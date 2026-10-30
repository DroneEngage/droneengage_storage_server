//! The SQLite store (database.js): same file, schema, migrations and
//! queries as the Node.js server, so existing databases open unchanged.
//!
//! Values reach SQL the way better-sqlite3 binds JavaScript values (`bind`):
//! every number is a REAL (5 stored in a TEXT column reads back as '5.0'),
//! undefined and NaN are NULL, booleans are rejected, an array argument is
//! spread into its elements and an object argument is a bag of named
//! parameters. Rows read back as better-sqlite3 returns them: INTEGER and
//! REAL as JavaScript numbers, columns in table order.

use rusqlite::types::{Value as Sql, ValueRef};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::logger::Log;
use crate::util::{js_stringify, js_truthy};

/// A JavaScript argument: None is `undefined`.
pub type Arg = Option<Value>;

/// `undefined`
pub const U: Arg = None;

/// A JavaScript string argument.
pub fn s(p_text: &str) -> Arg {
    Some(Value::String(p_text.to_string()))
}

/// A JavaScript number argument.
pub fn n(p_value: f64) -> Arg {
    Some(num(p_value))
}

/// A JSON number for a double (non-finite values are JSON null, as
/// JSON.stringify writes them).
pub fn num(p_value: f64) -> Value {
    if p_value.fract() == 0.0 && p_value.abs() < 9_007_199_254_740_992.0 {
        return json!(p_value as i64);
    }
    serde_json::Number::from_f64(p_value).map(Value::Number).unwrap_or(Value::Null)
}

const CANNOT_BIND: &str = "SQLite3 can only bind numbers, strings, bigints, buffers, and null";

/// better-sqlite3's argument binding for a statement with p_count
/// positional parameters.
pub fn bind(p_count: usize, p_args: &[Arg]) -> Result<Vec<Sql>, String> {
    let mut v_out = Vec::with_capacity(p_count);
    let mut v_named = false;
    let push = |v_out: &mut Vec<Sql>, p_value: &Option<&Value>| -> Result<(), String> {
        let v = match p_value {
            None | Some(Value::Null) => Sql::Null,
            Some(Value::Number(x)) => {
                let f = crate::util::json_number_f64(x);
                if f.is_nan() {
                    Sql::Null
                } else {
                    Sql::Real(f)
                }
            }
            Some(Value::String(t)) => Sql::Text(t.clone()),
            Some(_) => return Err(CANNOT_BIND.into()),
        };
        if v_out.len() >= p_count {
            return Err("Too many parameter values were provided".into());
        }
        v_out.push(v);
        Ok(())
    };
    for v_arg in p_args {
        match v_arg {
            Some(Value::Array(a)) => {
                for e in a {
                    push(&mut v_out, &Some(e))?;
                }
            }
            Some(Value::Object(_)) => {
                if v_named {
                    return Err("You cannot specify named parameters in two different objects".into());
                }
                v_named = true;
            }
            Some(Value::Bool(_)) => return Err(CANNOT_BIND.into()),
            other => push(&mut v_out, &other.as_ref())?,
        }
    }
    if v_out.len() < p_count {
        return Err("Too few parameter values were provided".into());
    }
    Ok(v_out)
}

/// A column value as better-sqlite3 returns it.
fn read(p_value: ValueRef<'_>) -> Value {
    match p_value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => num(i as f64),
        ValueRef::Real(f) => num(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        // JSON.stringify(Buffer)
        ValueRef::Blob(b) => json!({ "type": "Buffer", "data": b }),
    }
}

/// The message of an SQLite error (better-sqlite3's error.message).
fn sql_err(p_err: rusqlite::Error) -> String {
    match p_err {
        rusqlite::Error::SqliteFailure(e, Some(m)) => {
            let _ = e;
            m
        }
        rusqlite::Error::SqliteFailure(e, None) => e.to_string(),
        e => e.to_string(),
    }
}

/// stmt.run() result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunResult {
    pub changes: u64,
    pub last_insert_rowid: i64,
}

/// A paginated listing for the dashboard.
pub struct Page {
    pub rows: Vec<Value>,
    pub total: f64,
    pub page: f64,
    pub limit: f64,
}

impl Page {
    /// `{rows, total, page, limit, totalPages}` merged into p_out.
    pub fn into_fields(self, p_out: &mut Map<String, Value>) {
        p_out.insert("rows".into(), Value::Array(self.rows));
        p_out.insert("total".into(), num(self.total));
        p_out.insert("page".into(), num(self.page));
        p_out.insert("limit".into(), num(self.limit));
        p_out.insert("totalPages".into(), num((self.total / self.limit).ceil()));
    }
}

/// The parse of JSON.parse(String(text)); Err leaves the raw value.
fn parse_json_text(p_value: &Value) -> Result<Value, String> {
    let v_text = crate::util::js_to_string(p_value);
    serde_json::from_str(&v_text).map_err(|e| e.to_string())
}

/// fn_parseMissionRow: `data` and `meta` parsed in place.
fn parse_mission_row(p_row: &mut Value) {
    let Value::Object(m) = p_row else { return };
    // a value that doesn't parse is left raw (JSON.parse(undefined) throws too)
    if let Some(v) = m.get("data").and_then(|d| parse_json_text(d).ok()) {
        m.insert("data".into(), v);
    }
    let v_meta = match m.get("meta") {
        Some(v) if js_truthy(Some(v)) => parse_json_text(v).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    m.insert("meta".into(), v_meta);
}

/// Paging arguments (`Math.max(1, page)`, `Math.min(500, Math.max(1, limit))`).
fn paging(p_page: f64, p_limit: f64) -> (f64, f64, f64) {
    let p = js_max(1.0, p_page);
    let l = js_min(500.0, js_max(1.0, p_limit));
    (p, l, (p - 1.0) * l)
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// The WHERE clause and its arguments for optional `col = ?` filters.
fn filters(p_filters: &[(&str, &Arg)]) -> (String, Vec<Arg>) {
    let mut v_where = Vec::new();
    let mut v_args = Vec::new();
    for (v_col, v_val) in p_filters {
        if js_truthy(v_val.as_ref()) {
            v_where.push(format!("{} = ?", v_col));
            v_args.push((*v_val).clone());
        }
    }
    (v_where.join(" AND "), v_args)
}

fn where_clause(p_parts: &[String]) -> String {
    let v: Vec<&String> = p_parts.iter().filter(|s| !s.is_empty()).collect();
    if v.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" AND "))
    }
}

pub struct Db {
    m_conn: Mutex<Connection>,
    m_path: PathBuf,
    m_log: Log,
}

impl Db {
    /// initialize(): opens (creating the directory) and migrates the database.
    pub fn open(p_path: &Path, p_log: Log) -> Result<Db, String> {
        if let Some(v_dir) = p_path.parent() {
            if !v_dir.as_os_str().is_empty() && !v_dir.exists() {
                std::fs::create_dir_all(v_dir).map_err(|e| e.to_string())?;
            }
        }
        let v_conn = Connection::open(p_path).map_err(sql_err)?;
        v_conn.pragma_update(None, "journal_mode", "WAL").map_err(sql_err)?;
        v_conn.pragma_update(None, "foreign_keys", "ON").map_err(sql_err)?;
        let v_db = Db { m_conn: Mutex::new(v_conn), m_path: p_path.to_path_buf(), m_log: p_log };
        v_db.m_log.info(&format!("Database initialized at {}", p_path.display()));
        v_db.create_tables();
        v_db.create_indexes();
        Ok(v_db)
    }

    /// An in-memory database with the full schema (tests).
    pub fn memory(p_log: Log) -> Db {
        let v_conn = Connection::open_in_memory().unwrap();
        v_conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        let v_db = Db { m_conn: Mutex::new(v_conn), m_path: PathBuf::new(), m_log: p_log };
        v_db.create_tables();
        v_db.create_indexes();
        v_db
    }

    pub fn path(&self) -> &Path {
        &self.m_path
    }

    fn exec(&self, p_sql: &str) -> Result<(), String> {
        self.m_conn.lock().unwrap().execute_batch(p_sql).map_err(sql_err)
    }

    /// stmt.run(...args)
    pub fn run(&self, p_sql: &str, p_args: &[Arg]) -> Result<RunResult, String> {
        let v_conn = self.m_conn.lock().unwrap();
        let mut v_stmt = v_conn.prepare_cached(p_sql).map_err(sql_err)?;
        let v_params = bind(v_stmt.parameter_count(), p_args)?;
        let v_changes = v_stmt.execute(rusqlite::params_from_iter(v_params)).map_err(sql_err)?;
        Ok(RunResult { changes: v_changes as u64, last_insert_rowid: v_conn.last_insert_rowid() })
    }

    /// stmt.all(...args)
    pub fn all(&self, p_sql: &str, p_args: &[Arg]) -> Result<Vec<Value>, String> {
        let v_conn = self.m_conn.lock().unwrap();
        let mut v_stmt = v_conn.prepare_cached(p_sql).map_err(sql_err)?;
        let v_params = bind(v_stmt.parameter_count(), p_args)?;
        let v_names: Vec<String> = v_stmt.column_names().iter().map(|s| s.to_string()).collect();
        let mut v_rows = v_stmt.query(rusqlite::params_from_iter(v_params)).map_err(sql_err)?;
        let mut v_out = Vec::new();
        while let Some(r) = v_rows.next().map_err(sql_err)? {
            let mut m = Map::new();
            for (i, v_name) in v_names.iter().enumerate() {
                m.insert(v_name.clone(), read(r.get_ref(i).map_err(sql_err)?));
            }
            v_out.push(Value::Object(m));
        }
        Ok(v_out)
    }

    /// stmt.get(...args); None is undefined.
    pub fn get(&self, p_sql: &str, p_args: &[Arg]) -> Result<Option<Value>, String> {
        Ok(self.all(p_sql, p_args)?.into_iter().next())
    }

    fn count(&self, p_sql: &str, p_args: &[Arg], p_col: &str) -> Result<f64, String> {
        let v_row = self.get(p_sql, p_args)?;
        Ok(v_row.and_then(|r| r.get(p_col).and_then(Value::as_f64)).unwrap_or(0.0))
    }

    fn create_tables(&self) {
        let v_tables = [
            "CREATE TABLE IF NOT EXISTS units (
        id TEXT PRIMARY KEY,
        name TEXT,
        comm_server_id TEXT,
        created_at INTEGER DEFAULT (strftime('%s', 'now')),
        updated_at INTEGER DEFAULT (strftime('%s', 'now'))
      )",
            "CREATE TABLE IF NOT EXISTS tasks (
        id TEXT PRIMARY KEY,
        unit_id TEXT NOT NULL,
        name TEXT,
        data TEXT NOT NULL,
        version INTEGER DEFAULT 1,
        created_at INTEGER DEFAULT (strftime('%s', 'now')),
        updated_at INTEGER DEFAULT (strftime('%s', 'now')),
        disabled INTEGER DEFAULT 0,
        FOREIGN KEY (unit_id) REFERENCES units(id) ON DELETE CASCADE
      )",
            "CREATE TABLE IF NOT EXISTS missions (
        id TEXT PRIMARY KEY,
        unit_id TEXT NOT NULL,
        account_id TEXT NOT NULL,
        name TEXT,
        data TEXT NOT NULL,
        version INTEGER DEFAULT 1,
        created_at INTEGER DEFAULT (strftime('%s', 'now')),
        updated_at INTEGER DEFAULT (strftime('%s', 'now')),
        FOREIGN KEY (unit_id) REFERENCES units(id) ON DELETE CASCADE
      )",
            "CREATE TABLE IF NOT EXISTS news (
        id TEXT PRIMARY KEY,
        scope TEXT NOT NULL,
        account_id TEXT,
        title TEXT,
        body TEXT NOT NULL,
        priority INTEGER DEFAULT 0,
        author_id TEXT,
        expires_at INTEGER,
        disabled INTEGER DEFAULT 0,
        created_at INTEGER DEFAULT (strftime('%s', 'now')),
        updated_at INTEGER DEFAULT (strftime('%s', 'now'))
      )",
            "CREATE TABLE IF NOT EXISTS offline_queue (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        unit_id TEXT NOT NULL,
        task_id TEXT,
        message_type INTEGER NOT NULL,
        message_data TEXT NOT NULL,
        created_at INTEGER DEFAULT (strftime('%s', 'now')),
        priority INTEGER DEFAULT 0,
        status TEXT DEFAULT 'pending',
        FOREIGN KEY (unit_id) REFERENCES units(id) ON DELETE CASCADE
      )",
            "CREATE TABLE IF NOT EXISTS access_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        unit_id TEXT,
        action TEXT NOT NULL,
        resource_type TEXT,
        resource_id TEXT,
        comm_server_id TEXT,
        created_at INTEGER DEFAULT (strftime('%s', 'now'))
      )",
        ];
        for v_sql in v_tables {
            if let Err(e) = self.exec(v_sql) {
                self.m_log.error(&format!("Error creating table: {}", e));
            }
        }
        self.m_log.info("Database tables created");

        let v_migrations = [
            ("ALTER TABLE missions ADD COLUMN account_id TEXT NOT NULL DEFAULT '_unknown_'", "account_id"),
            ("ALTER TABLE missions ADD COLUMN kind TEXT NOT NULL DEFAULT 'de_plan'", "kind"),
            ("ALTER TABLE missions ADD COLUMN size INTEGER NOT NULL DEFAULT 0", "size"),
            ("ALTER TABLE missions ADD COLUMN meta TEXT", "meta"),
        ];
        for (v_sql, v_col) in v_migrations {
            match self.exec(v_sql) {
                Ok(()) => self.m_log.info(&format!("Migration: added {} column to missions table", v_col)),
                Err(e) if e.contains("duplicate column") => {}
                Err(e) => self.m_log.error(&format!("Migration: failed to add {} to missions: {}", v_col, e)),
            }
        }

        if let Err(e) = self.exec("UPDATE missions SET size = length(data) WHERE size = 0") {
            self.m_log.error(&format!("Migration: failed to backfill missions.size: {}", e));
        }

        let v_backfill = || -> Result<usize, String> {
            let v_orphans = self.all("SELECT rowid FROM missions WHERE id IS NULL", &[])?;
            for v_row in &v_orphans {
                self.run(
                    "UPDATE missions SET id = ? WHERE rowid = ?",
                    &[s(&crate::util::uuid_v4()), v_row.get("rowid").cloned()],
                )?;
            }
            Ok(v_orphans.len())
        };
        match v_backfill() {
            Ok(0) => {}
            Ok(c) => self.m_log.info(&format!("Migration: backfilled id for {} mission row(s)", c)),
            Err(e) => self.m_log.error(&format!("Migration: failed to backfill missions.id: {}", e)),
        }
    }

    fn create_indexes(&self) {
        let v_indexes = [
            "CREATE INDEX IF NOT EXISTS idx_tasks_unit_id ON tasks(unit_id)",
            "CREATE INDEX IF NOT EXISTS idx_tasks_disabled ON tasks(disabled)",
            "CREATE INDEX IF NOT EXISTS idx_offline_queue_unit_id ON offline_queue(unit_id)",
            "CREATE INDEX IF NOT EXISTS idx_offline_queue_status ON offline_queue(status)",
            "CREATE INDEX IF NOT EXISTS idx_offline_queue_priority ON offline_queue(priority)",
            "CREATE INDEX IF NOT EXISTS idx_access_log_unit_id ON access_log(unit_id)",
            "CREATE INDEX IF NOT EXISTS idx_access_log_created_at ON access_log(created_at)",
            "CREATE INDEX IF NOT EXISTS idx_missions_unit_id ON missions(unit_id)",
            "CREATE INDEX IF NOT EXISTS idx_missions_account_id ON missions(account_id)",
            "CREATE INDEX IF NOT EXISTS idx_missions_account_kind ON missions(account_id, kind, updated_at)",
            "CREATE INDEX IF NOT EXISTS idx_news_scope ON news(scope)",
            "CREATE INDEX IF NOT EXISTS idx_news_account_id ON news(account_id)",
            "CREATE INDEX IF NOT EXISTS idx_news_disabled ON news(disabled)",
            "CREATE INDEX IF NOT EXISTS idx_news_created_at ON news(created_at)",
        ];
        for v_sql in v_indexes {
            if let Err(e) = self.exec(v_sql) {
                self.m_log.error(&format!("Error creating index: {}", e));
            }
        }
        self.m_log.info("Database indexes created");
    }

    // ─── Units ──────────────────────────────────────────────────────────

    pub fn upsert_unit(&self, p_unit_id: &Arg, p_name: &Arg, p_comm_server_id: &Arg) -> Result<RunResult, String> {
        self.run(
            "
      INSERT INTO units (id, name, comm_server_id)
      VALUES (?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        name = excluded.name,
        comm_server_id = excluded.comm_server_id,
        updated_at = strftime('%s', 'now')
    ",
            &[p_unit_id.clone(), p_name.clone(), p_comm_server_id.clone()],
        )
    }

    pub fn get_unit(&self, p_unit_id: &Arg) -> Result<Option<Value>, String> {
        self.get("SELECT * FROM units WHERE id = ?", std::slice::from_ref(p_unit_id))
    }

    // ─── Tasks ──────────────────────────────────────────────────────────

    /// saveTask: p_data is stringified (undefined stays undefined -> NULL).
    pub fn save_task(&self, p_task_id: &Arg, p_unit_id: &Arg, p_name: &Arg, p_data: &Arg) -> Result<RunResult, String> {
        let v_data = p_data.as_ref().map(|d| Value::String(js_stringify(d)));
        self.run(
            "
      INSERT INTO tasks (id, unit_id, name, data)
      VALUES (?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        unit_id = excluded.unit_id,
        name = excluded.name,
        data = excluded.data,
        version = version + 1,
        updated_at = strftime('%s', 'now')
    ",
            &[p_task_id.clone(), p_unit_id.clone(), p_name.clone(), v_data],
        )
    }

    /// loadTasks: enabled tasks of a unit, data parsed (a bad value throws).
    pub fn load_tasks(&self, p_unit_id: &Arg) -> Result<Vec<Value>, String> {
        let v_rows = self.all(
            "
      SELECT * FROM tasks
      WHERE unit_id = ? AND disabled = 0
      ORDER BY created_at DESC
    ",
            std::slice::from_ref(p_unit_id),
        )?;
        v_rows
            .into_iter()
            .map(|mut t| {
                let v_data = parse_json_text(t.get("data").unwrap_or(&Value::Null))?;
                t["data"] = v_data;
                Ok(t)
            })
            .collect()
    }

    pub fn delete_task(&self, p_task_id: &Arg) -> Result<RunResult, String> {
        self.run("DELETE FROM tasks WHERE id = ?", std::slice::from_ref(p_task_id))
    }

    pub fn disable_task(&self, p_task_id: &Arg) -> Result<RunResult, String> {
        self.run(
            "
      UPDATE tasks
      SET disabled = 1, updated_at = strftime('%s', 'now')
      WHERE id = ?
    ",
            std::slice::from_ref(p_task_id),
        )
    }

    // ─── Missions ───────────────────────────────────────────────────────

    /// The owning account of a mission id; None when the id does not exist.
    pub fn get_mission_account_id(&self, p_mission_id: &Arg) -> Result<Option<Value>, String> {
        Ok(self
            .get("SELECT account_id FROM missions WHERE id = ?", std::slice::from_ref(p_mission_id))?
            .map(|r| r.get("account_id").cloned().unwrap_or(Value::Null)))
    }

    /// saveMission: updates a same-account id, inserts a new one, refuses a
    /// foreign one. p_data must be defined (Buffer.byteLength of undefined
    /// throws before this in the JavaScript server).
    #[allow(clippy::too_many_arguments)]
    pub fn save_mission(
        &self,
        p_mission_id: &Arg,
        p_unit_id: &Arg,
        p_account_id: &Arg,
        p_name: &Arg,
        p_data: &Arg,
        p_kind: &str,
        p_meta: &Arg,
    ) -> Result<RunResult, String> {
        let Some(v_data) = p_data.as_ref().map(js_stringify) else {
            return Err(
                "The \"string\" argument must be of type string or an instance of Buffer or ArrayBuffer. Received undefined"
                    .into(),
            );
        };
        let v_meta = match p_meta {
            None | Some(Value::Null) => Some(Value::Null),
            Some(m) => Some(Value::String(js_stringify(m))),
        };
        let v_size = v_data.len() as f64;
        let v_owner = self.get_mission_account_id(p_mission_id)?;
        if let Some(v_owner) = v_owner {
            // ownerAccount !== null: a NULL account_id reads as null
            if !v_owner.is_null() {
                if !crate::util::js_strict_eq(Some(&v_owner), p_account_id.as_ref()) {
                    return Err("mission id belongs to another account".into());
                }
                return self.run(
                    "
        UPDATE missions SET
          unit_id = ?, account_id = ?, name = ?, data = ?,
          kind = ?, size = ?, meta = ?,
          version = version + 1,
          updated_at = strftime('%s', 'now')
        WHERE id = ? AND account_id = ?
      ",
                    &[
                        p_unit_id.clone(),
                        p_account_id.clone(),
                        p_name.clone(),
                        s(&v_data),
                        s(p_kind),
                        n(v_size),
                        v_meta,
                        p_mission_id.clone(),
                        p_account_id.clone(),
                    ],
                );
            }
        }
        self.run(
            "
      INSERT INTO missions (id, unit_id, account_id, name, data, kind, size, meta)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)
    ",
            &[
                p_mission_id.clone(),
                p_unit_id.clone(),
                p_account_id.clone(),
                p_name.clone(),
                s(&v_data),
                s(p_kind),
                n(v_size),
                v_meta,
            ],
        )
    }

    pub fn load_missions(&self, p_unit_id: &Arg, p_account_id: &Arg) -> Result<Vec<Value>, String> {
        let mut v_rows = self.all(
            "
      SELECT * FROM missions
      WHERE unit_id = ? AND account_id = ?
      ORDER BY created_at DESC
    ",
            &[p_unit_id.clone(), p_account_id.clone()],
        )?;
        v_rows.iter_mut().for_each(parse_mission_row);
        Ok(v_rows)
    }

    /// listMissionsMeta: metadata rows (no data), newest first, max 200.
    pub fn list_missions_meta(
        &self,
        p_account_id: &Arg,
        p_kind: &Arg,
        p_unit_id: &Arg,
        p_before: &Arg,
    ) -> Result<Vec<Value>, String> {
        let mut v_where = vec!["account_id = ?".to_string()];
        let mut v_args = vec![p_account_id.clone()];
        if js_truthy(p_kind.as_ref()) {
            v_where.push("kind = ?".into());
            v_args.push(p_kind.clone());
        }
        if js_truthy(p_unit_id.as_ref()) {
            v_where.push("unit_id = ?".into());
            v_args.push(p_unit_id.clone());
        }
        if !crate::util::js_nullish(p_before.as_ref()) {
            v_where.push("updated_at < ?".into());
            v_args.push(p_before.clone());
        }
        v_args.push(n(200.0));
        let v_rows = self.all(
            &format!(
                "
      SELECT id, unit_id, name, kind, size, meta, version, created_at, updated_at
      FROM missions
      WHERE {}
      ORDER BY updated_at DESC
      LIMIT ?
    ",
                v_where.join(" AND ")
            ),
            &v_args,
        )?;
        v_rows
            .into_iter()
            .map(|m| {
                let v_meta = match m.get("meta") {
                    Some(v) if js_truthy(Some(v)) => parse_json_text(v)?,
                    _ => Value::Null,
                };
                let g = |k: &str| m.get(k).cloned().unwrap_or(Value::Null);
                Ok(json!({
                    "missionId": g("id"),
                    "unitId": g("unit_id"),
                    "name": g("name"),
                    "kind": g("kind"),
                    "size": g("size"),
                    "meta": v_meta,
                    "version": g("version"),
                    "created_at": g("created_at"),
                    "updated_at": g("updated_at"),
                }))
            })
            .collect()
    }

    pub fn get_mission(&self, p_mission_id: &Arg, p_account_id: &Arg) -> Result<Option<Value>, String> {
        let mut v_row = self.get(
            "SELECT * FROM missions WHERE id = ? AND account_id = ?",
            &[p_mission_id.clone(), p_account_id.clone()],
        )?;
        if let Some(r) = v_row.as_mut() {
            parse_mission_row(r);
        }
        Ok(v_row)
    }

    pub fn delete_mission(&self, p_mission_id: &Arg, p_account_id: &Arg) -> Result<RunResult, String> {
        self.run("DELETE FROM missions WHERE id = ? AND account_id = ?", &[p_mission_id.clone(), p_account_id.clone()])
    }

    pub fn delete_mission_by_id(&self, p_mission_id: &Arg) -> Result<RunResult, String> {
        self.run("DELETE FROM missions WHERE id = ?", std::slice::from_ref(p_mission_id))
    }

    // ─── News ───────────────────────────────────────────────────────────

    /// saveNews: `accountId || null`, `priority || 0`, `authorId || null`,
    /// `expiresAt || null`.
    #[allow(clippy::too_many_arguments)]
    pub fn save_news(
        &self,
        p_news_id: &Arg,
        p_scope: &Arg,
        p_account_id: &Arg,
        p_title: &Arg,
        p_body: &Arg,
        p_priority: &Arg,
        p_author_id: &Arg,
        p_expires_at: &Arg,
    ) -> Result<RunResult, String> {
        let or = |v: &Arg, d: Value| if js_truthy(v.as_ref()) { v.clone() } else { Some(d) };
        self.run(
            "
      INSERT INTO news (id, scope, account_id, title, body, priority, author_id, expires_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        scope = excluded.scope,
        account_id = excluded.account_id,
        title = excluded.title,
        body = excluded.body,
        priority = excluded.priority,
        expires_at = excluded.expires_at,
        updated_at = strftime('%s', 'now')
    ",
            &[
                p_news_id.clone(),
                p_scope.clone(),
                or(p_account_id, Value::Null),
                p_title.clone(),
                p_body.clone(),
                or(p_priority, json!(0)),
                or(p_author_id, Value::Null),
                or(p_expires_at, Value::Null),
            ],
        )
    }

    pub fn load_news(&self, p_account_id: &Arg) -> Result<Vec<Value>, String> {
        self.all(
            "
      SELECT * FROM news
      WHERE disabled = 0
        AND (expires_at IS NULL OR expires_at > strftime('%s', 'now'))
        AND (scope = 'global' OR (scope = 'account' AND account_id = ?))
      ORDER BY created_at DESC
    ",
            std::slice::from_ref(p_account_id),
        )
    }

    pub fn get_news(&self, p_news_id: &Arg) -> Result<Option<Value>, String> {
        self.get("SELECT * FROM news WHERE id = ?", std::slice::from_ref(p_news_id))
    }

    pub fn disable_news(&self, p_news_id: &Arg) -> Result<RunResult, String> {
        self.run(
            "
      UPDATE news SET disabled = 1, updated_at = strftime('%s', 'now') WHERE id = ?
    ",
            std::slice::from_ref(p_news_id),
        )
    }

    pub fn enable_news(&self, p_news_id: &Arg) -> Result<RunResult, String> {
        self.run(
            "
      UPDATE news SET disabled = 0, updated_at = strftime('%s', 'now') WHERE id = ?
    ",
            std::slice::from_ref(p_news_id),
        )
    }

    pub fn delete_news(&self, p_news_id: &Arg) -> Result<RunResult, String> {
        self.run("DELETE FROM news WHERE id = ?", std::slice::from_ref(p_news_id))
    }

    // ─── Quotas ─────────────────────────────────────────────────────────

    pub fn count_news_by_account(&self, p_account_id: &Arg) -> Result<f64, String> {
        self.count(
            "SELECT COUNT(*) AS n FROM news
       WHERE scope = 'account' AND account_id = ? AND disabled = 0",
            std::slice::from_ref(p_account_id),
            "n",
        )
    }

    pub fn count_missions_by_account(&self, p_account_id: &Arg) -> Result<f64, String> {
        self.count("SELECT COUNT(*) AS n FROM missions WHERE account_id = ?", std::slice::from_ref(p_account_id), "n")
    }

    pub fn count_missions_by_unit(&self, p_unit_id: &Arg, p_account_id: &Arg) -> Result<f64, String> {
        self.count(
            "SELECT COUNT(*) AS n FROM missions WHERE unit_id = ? AND account_id = ?",
            &[p_unit_id.clone(), p_account_id.clone()],
            "n",
        )
    }

    pub fn count_missions_by_kind(&self, p_account_id: &Arg, p_kind: &str) -> Result<f64, String> {
        self.count(
            "SELECT COUNT(*) AS n FROM missions WHERE account_id = ? AND kind = ?",
            &[p_account_id.clone(), s(p_kind)],
            "n",
        )
    }

    // ─── Offline queue ──────────────────────────────────────────────────

    pub fn add_to_queue(
        &self,
        p_unit_id: &Arg,
        p_type: &Arg,
        p_data: &Value,
        p_task_id: &Arg,
        p_priority: f64,
    ) -> Result<RunResult, String> {
        self.run(
            "
      INSERT INTO offline_queue (unit_id, task_id, message_type, message_data, priority)
      VALUES (?, ?, ?, ?, ?)
    ",
            &[p_unit_id.clone(), p_task_id.clone(), p_type.clone(), s(&js_stringify(p_data)), n(p_priority)],
        )
    }

    pub fn get_queued_messages(&self, p_unit_id: &Arg, p_limit: f64) -> Result<Vec<Value>, String> {
        let v_rows = self.all(
            "
      SELECT * FROM offline_queue
      WHERE unit_id = ? AND status = 'pending'
      ORDER BY priority DESC, created_at ASC
      LIMIT ?
    ",
            &[p_unit_id.clone(), n(p_limit)],
        )?;
        v_rows
            .into_iter()
            .map(|mut m| {
                let v = parse_json_text(m.get("message_data").unwrap_or(&Value::Null))?;
                m["message_data"] = v;
                Ok(m)
            })
            .collect()
    }

    pub fn mark_message_delivered(&self, p_id: &Arg) -> Result<RunResult, String> {
        self.run(
            "
      UPDATE offline_queue
      SET status = 'delivered'
      WHERE id = ?
    ",
            std::slice::from_ref(p_id),
        )
    }

    /// Deletes delivered messages older than a day.
    pub fn cleanup_delivered_messages(&self) -> Result<RunResult, String> {
        let v_res = self.run(
            "
      DELETE FROM offline_queue
      WHERE status = 'delivered' AND created_at < strftime('%s', 'now') - 86400
    ",
            &[],
        )?;
        let v_text = format!("Cleaned up {} delivered queue messages", v_res.changes);
        if v_res.changes > 0 {
            self.m_log.info(&v_text);
        } else {
            self.m_log.debug(&v_text);
        }
        Ok(v_res)
    }

    pub fn get_queue_size(&self, p_unit_id: &Arg) -> Result<f64, String> {
        self.count(
            "
      SELECT COUNT(*) as count FROM offline_queue
      WHERE unit_id = ? AND status = 'pending'
    ",
            std::slice::from_ref(p_unit_id),
            "count",
        )
    }

    pub fn get_queue_stats(&self) -> Result<Value, String> {
        Ok(self
            .get(
                "
      SELECT
        COUNT(*) as totalMessages,
        COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0) as pendingMessages,
        COALESCE(COUNT(DISTINCT unit_id), 0) as affectedUnits
      FROM offline_queue
      WHERE status = 'pending'
    ",
                &[],
            )?
            .unwrap_or(Value::Null))
    }

    pub fn clear_queue(&self, p_unit_id: &Arg) -> Result<RunResult, String> {
        self.run(
            "
      DELETE FROM offline_queue WHERE unit_id = ?
    ",
            std::slice::from_ref(p_unit_id),
        )
    }

    // ─── Access log ─────────────────────────────────────────────────────

    pub fn log_access(
        &self,
        p_unit_id: &Arg,
        p_action: &str,
        p_type: &str,
        p_resource_id: &Arg,
        p_comm_server_id: &Arg,
    ) -> Result<RunResult, String> {
        self.run(
            "
      INSERT INTO access_log (unit_id, action, resource_type, resource_id, comm_server_id)
      VALUES (?, ?, ?, ?, ?)
    ",
            &[p_unit_id.clone(), s(p_action), s(p_type), p_resource_id.clone(), p_comm_server_id.clone()],
        )
    }

    /// clearAccessLog: deletes all rows, or those matching the filters.
    pub fn clear_access_log(&self, p_action: &Arg, p_unit_id: &Arg) -> Result<u64, String> {
        let (v_where, v_args) = filters(&[("action", p_action), ("unit_id", p_unit_id)]);
        Ok(self.run(&format!("DELETE FROM access_log{}", where_clause(&[v_where])), &v_args)?.changes)
    }

    // ─── Dashboard pages ────────────────────────────────────────────────

    fn page(
        &self,
        p_table: &str,
        p_where: String,
        p_args: Vec<Arg>,
        p_order: &str,
        p_page: f64,
        p_limit: f64,
    ) -> Result<Page, String> {
        let (p, l, v_offset) = paging(p_page, p_limit);
        let v_total = self.count(&format!("SELECT COUNT(*) as count FROM {}{}", p_table, p_where), &p_args, "count")?;
        let mut v_args = p_args;
        v_args.push(n(l));
        v_args.push(n(v_offset));
        let v_rows =
            self.all(&format!("SELECT * FROM {}{} ORDER BY {} LIMIT ? OFFSET ?", p_table, p_where, p_order), &v_args)?;
        Ok(Page { rows: v_rows, total: v_total, page: p, limit: l })
    }

    pub fn get_access_log_page(
        &self,
        p_page: f64,
        p_limit: f64,
        p_action: &Arg,
        p_unit_id: &Arg,
    ) -> Result<Page, String> {
        let (v_where, v_args) = filters(&[("action", p_action), ("unit_id", p_unit_id)]);
        self.page("access_log", where_clause(&[v_where]), v_args, "created_at DESC, id DESC", p_page, p_limit)
    }

    pub fn get_units_page(&self, p_page: f64, p_limit: f64, p_search: &Arg) -> Result<Page, String> {
        let (v_where, v_args) = match p_search {
            Some(v) if js_truthy(Some(v)) => {
                let v_like = s(&format!("%{}%", crate::util::js_to_string(v)));
                (
                    " WHERE id LIKE ? OR name LIKE ? OR comm_server_id LIKE ?".to_string(),
                    vec![v_like.clone(), v_like.clone(), v_like],
                )
            }
            _ => (String::new(), Vec::new()),
        };
        let mut v_page = self.page("units", v_where, v_args, "updated_at DESC", p_page, p_limit)?;
        for u in v_page.rows.iter_mut() {
            let v_id = u.get("id").cloned();
            let c = self
                .get(
                    "
      SELECT
        (SELECT COUNT(*) FROM tasks WHERE unit_id = ?) AS tasks,
        (SELECT COUNT(*) FROM tasks WHERE unit_id = ? AND disabled = 1) AS disabledTasks,
        (SELECT COUNT(*) FROM missions WHERE unit_id = ?) AS missions,
        (SELECT COUNT(*) FROM offline_queue WHERE unit_id = ? AND status = 'pending') AS pendingQueue
    ",
                    &[v_id.clone(), v_id.clone(), v_id.clone(), v_id],
                )?
                .unwrap_or(Value::Null);
            if let (Value::Object(um), Value::Object(cm)) = (u, c) {
                for (k, v) in cm {
                    um.insert(k, v);
                }
            }
        }
        Ok(v_page)
    }

    pub fn get_tasks_page(
        &self,
        p_page: f64,
        p_limit: f64,
        p_unit_id: &Arg,
        p_include_disabled: bool,
    ) -> Result<Page, String> {
        let (v_where, v_args) = filters(&[("unit_id", p_unit_id)]);
        let v_disabled = if p_include_disabled { String::new() } else { "disabled = 0".into() };
        let mut v_page =
            self.page("tasks", where_clause(&[v_where, v_disabled]), v_args, "updated_at DESC", p_page, p_limit)?;
        for t in v_page.rows.iter_mut() {
            if let Some(v) = t.get("data").and_then(|d| parse_json_text(d).ok()) {
                t["data"] = v;
            }
        }
        Ok(v_page)
    }

    pub fn get_missions_page(
        &self,
        p_page: f64,
        p_limit: f64,
        p_unit_id: &Arg,
        p_account_id: &Arg,
        p_kind: &Arg,
    ) -> Result<Page, String> {
        let (v_where, v_args) = filters(&[("unit_id", p_unit_id), ("account_id", p_account_id), ("kind", p_kind)]);
        let mut v_page = self.page("missions", where_clause(&[v_where]), v_args, "updated_at DESC", p_page, p_limit)?;
        v_page.rows.iter_mut().for_each(parse_mission_row);
        Ok(v_page)
    }

    pub fn get_queue_page(&self, p_page: f64, p_limit: f64, p_unit_id: &Arg, p_status: &Arg) -> Result<Page, String> {
        let (v_where, v_args) = filters(&[("unit_id", p_unit_id), ("status", p_status)]);
        let mut v_page =
            self.page("offline_queue", where_clause(&[v_where]), v_args, "created_at DESC, id DESC", p_page, p_limit)?;
        for q in v_page.rows.iter_mut() {
            if let Some(v) = q.get("message_data").and_then(|d| parse_json_text(d).ok()) {
                q["message_data"] = v;
            }
        }
        Ok(v_page)
    }

    pub fn get_mission_accounts(&self) -> Result<Vec<Value>, String> {
        Ok(self
            .all("SELECT DISTINCT account_id FROM missions ORDER BY account_id ASC", &[])?
            .into_iter()
            .map(|r| r.get("account_id").cloned().unwrap_or(Value::Null))
            .collect())
    }

    pub fn get_news_page(
        &self,
        p_page: f64,
        p_limit: f64,
        p_scope: &Arg,
        p_account_id: &Arg,
        p_include_disabled: bool,
    ) -> Result<Page, String> {
        let (v_where, v_args) = filters(&[("scope", p_scope), ("account_id", p_account_id)]);
        let v_disabled = if p_include_disabled { String::new() } else { "disabled = 0".into() };
        self.page("news", where_clause(&[v_where, v_disabled]), v_args, "created_at DESC", p_page, p_limit)
    }

    pub fn get_news_accounts(&self) -> Result<Vec<Value>, String> {
        Ok(self
            .all(
                "SELECT DISTINCT account_id FROM news WHERE scope = 'account' AND account_id IS NOT NULL ORDER BY account_id ASC",
                &[],
            )?
            .into_iter()
            .map(|r| r.get("account_id").cloned().unwrap_or(Value::Null))
            .collect())
    }

    pub fn get_all_unit_ids(&self) -> Result<Vec<Value>, String> {
        Ok(self
            .all(
                "
      SELECT id AS unit_id FROM units
      UNION SELECT unit_id FROM tasks
      UNION SELECT unit_id FROM missions
      UNION SELECT unit_id FROM offline_queue
      ORDER BY unit_id ASC
    ",
                &[],
            )?
            .into_iter()
            .map(|r| r.get("unit_id").cloned().unwrap_or(Value::Null))
            .collect())
    }

    /// getStats: table counts and the database file size.
    pub fn get_stats(&self) -> Result<Value, String> {
        let c = |q: &str| self.count(q, &[], "count").map(num);
        let v_size = std::fs::metadata(&self.m_path)
            .map_err(|_| format!("ENOENT: no such file or directory, stat '{}'", self.m_path.display()))?;
        Ok(json!({
            "units": c("SELECT COUNT(*) as count FROM units")?,
            "tasks": c("SELECT COUNT(*) as count FROM tasks")?,
            "disabledTasks": c("SELECT COUNT(*) as count FROM tasks WHERE disabled = 1")?,
            "missions": c("SELECT COUNT(*) as count FROM missions")?,
            "news": c("SELECT COUNT(*) as count FROM news")?,
            "activeNews": c("SELECT COUNT(*) as count FROM news WHERE disabled = 0 AND (expires_at IS NULL OR expires_at > strftime('%s', 'now'))")?,
            "queuedMessages": c("SELECT COUNT(*) as count FROM offline_queue WHERE status = 'pending'")?,
            "dbSize": num(v_size.len() as f64),
        }))
    }

    /// close(): flushes the WAL into the main file.
    pub fn close(&self) {
        let _ = self.exec("PRAGMA wal_checkpoint(TRUNCATE)");
        self.m_log.info("Database connection closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_like_better_sqlite3() {
        let a = |v: Value| Some(v);
        assert_eq!(bind(2, &[n(5.0), U]).unwrap(), vec![Sql::Real(5.0), Sql::Null]);
        assert_eq!(bind(1, &[a(json!(true))]).unwrap_err(), CANNOT_BIND);
        assert_eq!(bind(3, &[a(json!([1, 2])), n(3.0)]).unwrap().len(), 3);
        assert_eq!(bind(3, &[a(json!([[1]])), n(3.0)]).unwrap_err(), CANNOT_BIND);
        assert_eq!(
            bind(3, &[a(json!({"x": 1})), n(2.0), n(3.0)]).unwrap_err(),
            "Too few parameter values were provided"
        );
        assert_eq!(bind(3, &[n(1.0), n(2.0), n(3.0), a(json!({}))]).unwrap().len(), 3);
        assert_eq!(
            bind(3, &[a(json!({})), a(json!({})), a(json!(true))]).unwrap_err(),
            "You cannot specify named parameters in two different objects"
        );
        assert_eq!(
            bind(3, &[n(1.0), n(2.0), n(3.0), n(4.0), a(json!(true))]).unwrap_err(),
            "Too many parameter values were provided"
        );
        assert_eq!(bind(3, &[n(1.0), a(json!(true))]).unwrap_err(), CANNOT_BIND);
        assert_eq!(bind(1, &[n(f64::NAN)]).unwrap(), vec![Sql::Null]);
    }

    #[test]
    fn numbers_are_stored_as_real() {
        let v_db = Db::memory(Log::silent());
        v_db.upsert_unit(&n(5.0), &U, &s("C")).unwrap();
        let v_unit = v_db.get_unit(&n(5.0)).unwrap().unwrap();
        assert_eq!(v_unit["id"], json!("5.0"), "a number id reads back as better-sqlite3 stored it");
    }
}

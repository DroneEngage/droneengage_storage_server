//! Message handlers (messageHandlers.js): tasks, missions, news and the
//! unit-online queue flush, keyed by the message type `mt`.
//!
//! Each handler reads its fields from `ms` (or the message itself), runs
//! the same database calls in the same order as the JavaScript handler and
//! replies `{mt, ms, success, timestamp, rid?, error?}`. A failure anywhere
//! (a JavaScript TypeError, an SQLite error) becomes `ms.s = 'ERROR:<msg>'`.

use serde_json::{json, Map, Value};
use std::sync::Arc;

use crate::db::{s, Arg};
use crate::js::{destructure, element, iterate, join, prop};
use crate::server::Storage;
use crate::util::{
    field, js_interp, js_key, js_number, js_number_fmt, js_strict_eq, js_stringify, js_truthy, now_ms, uuid_v4,
};

pub const LOAD_TASKS: u32 = 9001;
pub const SAVE_TASKS: u32 = 9002;
pub const DELETE_TASKS: u32 = 9003;
pub const DISABLE_TASKS: u32 = 9004;
pub const UNIT_ONLINE: u32 = 9009;
pub const LOAD_MISSION: u32 = 9010;
pub const SAVE_MISSION: u32 = 9011;
pub const DELETE_MISSION: u32 = 9012;
pub const LOAD_NEWS: u32 = 9015;
pub const SAVE_NEWS: u32 = 9016;
pub const DELETE_NEWS: u32 = 9017;
pub const NEWS_PUSH: u32 = 9018;

/// Allowed mission kinds (P1F-01).
pub const MISSION_KINDS: [&str; 5] = ["de_plan", "de_operation", "de_package", "de_journal", "de_opgraph"];
pub const MISSION_META_MAX_BYTES: usize = 2048;
pub const MISSION_DATA_MAX_BYTES_DEFAULT: f64 = 1024.0 * 1024.0;

/// An `ms` object under construction; an undefined value is left out.
struct Ms(Map<String, Value>);

impl Ms {
    fn new(p_s: impl Into<String>) -> Ms {
        let mut m = Map::new();
        m.insert("s".into(), Value::String(p_s.into()));
        Ms(m)
    }

    fn empty() -> Ms {
        Ms(Map::new())
    }

    fn k(mut self, p_key: &str, p_value: Value) -> Ms {
        self.0.insert(p_key.into(), p_value);
        self
    }

    fn o(mut self, p_key: &str, p_value: &Arg) -> Ms {
        if let Some(v) = p_value {
            self.0.insert(p_key.into(), v.clone());
        }
        self
    }

    fn done(self) -> Value {
        Value::Object(self.0)
    }
}

/// buildResponseEnvelope: echoes the request id `rid` when present.
pub fn envelope(p_message: &Value, p_mt: u32, p_ms: Value, p_success: bool, p_error: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("mt".into(), json!(p_mt));
    m.insert("ms".into(), p_ms);
    m.insert("success".into(), json!(p_success));
    m.insert("timestamp".into(), json!(now_ms()));
    if let Some(v_rid) = field(p_message, "rid").filter(|v| !v.is_null()) {
        m.insert("rid".into(), v_rid.clone());
    }
    if let Some(e) = p_error {
        m.insert("error".into(), json!(e));
    }
    Value::Object(m)
}

/// getPayload: `ms` when it is an object (or array), else the message.
fn payload(p_message: &Value) -> &Value {
    match field(p_message, "ms") {
        Some(v @ (Value::Object(_) | Value::Array(_))) => v,
        _ => p_message,
    }
}

fn get(p_obj: &Value, p_key: &str) -> Arg {
    field(p_obj, p_key).cloned()
}

/// `${x}` of an argument.
fn i(p_value: &Arg) -> String {
    js_interp(p_value.as_ref())
}

/// `${n}` of a count.
fn c(p_value: f64) -> String {
    js_number_fmt(p_value)
}

pub struct Handlers {
    m_storage: Arc<Storage>,
}

impl Handlers {
    pub fn new(p_storage: Arc<Storage>) -> Handlers {
        Handlers { m_storage: p_storage }
    }

    fn st(&self) -> &Storage {
        &self.m_storage
    }

    /// The handler for `mt` (`handlers[message.mt]`: the key is String(mt)).
    pub fn lookup(p_mt: Option<&Value>) -> Option<u32> {
        let v_key = js_key(p_mt);
        [
            LOAD_TASKS,
            SAVE_TASKS,
            DELETE_TASKS,
            DISABLE_TASKS,
            UNIT_ONLINE,
            LOAD_MISSION,
            SAVE_MISSION,
            DELETE_MISSION,
            LOAD_NEWS,
            SAVE_NEWS,
            DELETE_NEWS,
        ]
        .into_iter()
        .find(|t| t.to_string() == v_key)
    }

    pub fn handle(&self, p_type: u32, p_conn: u64, p_message: &Value) {
        match p_type {
            LOAD_TASKS => self.load_tasks(p_conn, p_message),
            SAVE_TASKS => self.save_tasks(p_conn, p_message),
            DELETE_TASKS => self.delete_tasks(p_conn, p_message, false),
            DISABLE_TASKS => self.delete_tasks(p_conn, p_message, true),
            UNIT_ONLINE => self.unit_online(p_conn, p_message),
            LOAD_MISSION => self.load_mission(p_conn, p_message),
            SAVE_MISSION => self.save_mission(p_conn, p_message),
            DELETE_MISSION => self.delete_mission(p_conn, p_message),
            LOAD_NEWS => self.load_news(p_conn, p_message),
            SAVE_NEWS => self.save_news(p_conn, p_message),
            DELETE_NEWS => self.delete_news(p_conn, p_message),
            _ => {}
        }
    }

    fn reply(&self, p_conn: u64, p_message: &Value, p_mt: u32, p_ms: Ms, p_success: bool, p_error: Option<&str>) {
        self.st().send(p_conn, &envelope(p_message, p_mt, p_ms.done(), p_success, p_error));
    }

    fn fail(&self, p_conn: u64, p_message: &Value, p_mt: u32, p_err: &str, p_ms: impl FnOnce(Ms) -> Ms) {
        self.reply(p_conn, p_message, p_mt, p_ms(Ms::new(format!("ERROR:{}", p_err))), false, Some(p_err));
    }

    fn load_tasks(&self, p_conn: u64, p_message: &Value) {
        let v_payload = payload(p_message);
        let v_unit = get(v_payload, "unitId");
        let v_cs = self.st().comm_server_id(p_conn);
        let v_res = (|| -> Result<(), String> {
            self.st().log.debug(&format!("LoadTasks request for unit {} from comm server {}", i(&v_unit), i(&v_cs)));
            let v_tasks = self.st().db.load_tasks(&v_unit)?;
            self.st().db.log_access(&v_unit, "load", "task", &Some(Value::Null), &v_cs)?;
            let v_count = v_tasks.len();
            self.reply(
                p_conn,
                p_message,
                LOAD_TASKS,
                Ms::new("OK").k("tasks", Value::Array(v_tasks)).o("unitId", &v_unit),
                true,
                None,
            );
            self.st().log.debug(&format!("Loaded {} tasks for unit {}", v_count, i(&v_unit)));
            Ok(())
        })();
        if let Err(e) = v_res {
            self.st().log.error(&format!("Error loading tasks for unit {}: {}", i(&v_unit), e));
            self.fail(p_conn, p_message, LOAD_TASKS, &e, |m| m.o("unitId", &v_unit));
        }
    }

    fn save_tasks(&self, p_conn: u64, p_message: &Value) {
        let v_payload = payload(p_message);
        let v_unit = get(v_payload, "unitId");
        let v_tasks = get(v_payload, "tasks");
        let v_cs = self.st().comm_server_id(p_conn);
        let v_res = (|| -> Result<(), String> {
            let v_len = prop(v_tasks.as_ref(), "length")?;
            self.st().log.info(&format!("SaveTasks request for unit {} with {} tasks", i(&v_unit), i(&v_len)));
            self.st().db.upsert_unit(&v_unit, &Some(Value::Null), &v_cs)?;
            let mut v_saved = Vec::new();
            for v_task in iterate(v_tasks.as_ref(), "tasks")? {
                let f = destructure(&v_task, "task", &["taskId", "name", "data"])?;
                self.st().db.save_task(&f[0], &v_unit, &f[1], &f[2])?;
                v_saved.push(element(f[0].clone()));
            }
            self.st().db.log_access(&v_unit, "save", "task", &s(&join(&v_saved)), &v_cs)?;
            let v_count = v_saved.len();
            self.reply(
                p_conn,
                p_message,
                SAVE_TASKS,
                Ms::new("OK:save").k("savedTasks", Value::Array(v_saved)).o("unitId", &v_unit),
                true,
                None,
            );
            self.st().log.info(&format!("Saved {} tasks for unit {}", v_count, i(&v_unit)));
            Ok(())
        })();
        if let Err(e) = v_res {
            self.st().log.error(&format!("Error saving tasks for unit {}: {}", i(&v_unit), e));
            self.fail(p_conn, p_message, SAVE_TASKS, &e, |m| m.o("unitId", &v_unit));
        }
    }

    /// DeleteTasks (9003), or DisableTasks (9004) with p_disable.
    fn delete_tasks(&self, p_conn: u64, p_message: &Value, p_disable: bool) {
        let (v_mt, v_name, v_done, v_ok, v_key, v_action, v_past) = if p_disable {
            (DISABLE_TASKS, "DisableTasks", "Disabled", "OK:disable", "disabledTasks", "disable", "disabling")
        } else {
            (DELETE_TASKS, "DeleteTasks", "Deleted", "OK:del", "deletedTasks", "delete", "deleting")
        };
        let v_payload = payload(p_message);
        let v_unit = get(v_payload, "unitId");
        let v_ids = get(v_payload, "taskIds");
        let v_cs = self.st().comm_server_id(p_conn);
        let v_res = (|| -> Result<(), String> {
            let v_len = prop(v_ids.as_ref(), "length")?;
            self.st().log.info(&format!("{} request for unit {} with {} tasks", v_name, i(&v_unit), i(&v_len)));
            let mut v_list = Vec::new();
            for v_id in iterate(v_ids.as_ref(), "taskIds")? {
                let v_arg = Some(v_id.clone());
                if p_disable {
                    self.st().db.disable_task(&v_arg)?;
                } else {
                    self.st().db.delete_task(&v_arg)?;
                }
                v_list.push(v_id);
            }
            self.st().db.log_access(&v_unit, v_action, "task", &s(&join(&v_list)), &v_cs)?;
            let v_count = v_list.len();
            self.reply(
                p_conn,
                p_message,
                v_mt,
                Ms::new(v_ok).k(v_key, Value::Array(v_list)).o("unitId", &v_unit),
                true,
                None,
            );
            self.st().log.info(&format!("{} {} tasks for unit {}", v_done, v_count, i(&v_unit)));
            Ok(())
        })();
        if let Err(e) = v_res {
            self.st().log.error(&format!("Error {} tasks for unit {}: {}", v_past, i(&v_unit), e));
            self.fail(p_conn, p_message, v_mt, &e, |m| m.o("unitId", &v_unit));
        }
    }

    fn unit_online(&self, p_conn: u64, p_message: &Value) {
        let v_payload = payload(p_message);
        let v_unit = get(v_payload, "unitId");
        let v_cs = self.st().comm_server_id(p_conn);
        self.st().log.info(&format!("Unit online event for unit {} from comm server {}", i(&v_unit), i(&v_cs)));
        let v_result = crate::offline_queue::process_queue_for_unit(self.st(), &v_unit, p_conn);
        let n = |p: Option<f64>| p.map(crate::db::num);
        self.reply(
            p_conn,
            p_message,
            UNIT_ONLINE,
            Ms::new("OK").o("unitId", &v_unit).o("processed", &n(v_result.processed)).o("failed", &n(v_result.failed)),
            true,
            None,
        );
        let f = |p: Option<f64>| p.map(c).unwrap_or_else(|| "undefined".into());
        self.st().log.info(&format!(
            "Unit online processed for unit {}: {} messages delivered, {} failed",
            i(&v_unit),
            f(v_result.processed),
            f(v_result.failed)
        ));
    }

    fn load_mission(&self, p_conn: u64, p_message: &Value) {
        let v_payload = payload(p_message);
        let [v_unit, v_mission, v_account, v_list, v_kind, v_before, v_rq] =
            ["unitId", "missionId", "accountId", "list", "kind", "before", "rq"].map(|k| get(v_payload, k));
        let v_cs = self.st().comm_server_id(p_conn);
        let v_or_null = |v: &Arg| if js_truthy(v.as_ref()) { v.clone() } else { Some(Value::Null) };
        let v_res = (|| -> Result<(), String> {
            if crate::util::is_true(v_list.as_ref()) {
                let v_missions = self.st().db.list_missions_meta(&v_account, &v_kind, &v_unit, &v_before)?;
                self.st().db.log_access(&v_or_null(&v_unit), "load", "mission", &Some(Value::Null), &v_cs)?;
                let v_count = v_missions.len();
                self.reply(
                    p_conn,
                    p_message,
                    LOAD_MISSION,
                    Ms::new("OK")
                        .k("list", json!(true))
                        .k("missions", Value::Array(v_missions))
                        .o("unitId", &v_or_null(&v_unit))
                        .o("rq", &v_rq),
                    true,
                    None,
                );
                self.st().log.info(&format!("Listed {} missions for account {}", v_count, i(&v_account)));
                return Ok(());
            }
            let v_suffix =
                if js_truthy(v_mission.as_ref()) { format!(", mission {}", i(&v_mission)) } else { String::new() };
            self.st().log.info(&format!(
                "LoadMission request for unit {}{} from comm server {}",
                i(&v_unit),
                v_suffix,
                i(&v_cs)
            ));
            let v_result: Arg = if js_truthy(v_mission.as_ref()) {
                self.st().db.get_mission(&v_mission, &v_account)?
            } else {
                Some(Value::Array(self.st().db.load_missions(&v_unit, &v_account)?))
            };
            self.st().db.log_access(&v_unit, "load", "mission", &v_or_null(&v_mission), &v_cs)?;
            self.reply(
                p_conn,
                p_message,
                LOAD_MISSION,
                Ms::new("OK").o("mission", &v_result).o("unitId", &v_unit).o("rq", &v_rq),
                true,
                None,
            );
            self.st().log.info(&format!("Loaded mission for unit {}", i(&v_unit)));
            Ok(())
        })();
        if let Err(e) = v_res {
            self.st().log.error(&format!("Error loading mission for unit {}: {}", i(&v_unit), e));
            self.fail(p_conn, p_message, LOAD_MISSION, &e, |m| m.o("unitId", &v_unit).o("rq", &v_rq));
        }
    }

    fn save_mission(&self, p_conn: u64, p_message: &Value) {
        let v_payload = payload(p_message);
        let [v_unit, v_account, v_name, v_data, v_kind, v_meta, v_rq] =
            ["unitId", "accountId", "name", "data", "kind", "meta", "rq"].map(|k| get(v_payload, k));
        let v_cs = self.st().comm_server_id(p_conn);
        let mut v_mission_id = get(v_payload, "missionId");
        let v_empty = |v: &Arg| match v {
            None | Some(Value::Null) => true,
            Some(Value::String(t)) => t.is_empty(),
            _ => false,
        };
        let v_res = (|| -> Result<(), String> {
            let v_kind_text = if v_empty(&v_kind) {
                Some("de_plan".to_string())
            } else {
                v_kind.as_ref().and_then(Value::as_str).map(str::to_string)
            };
            let v_kind_display = if v_empty(&v_kind) { "de_plan".to_string() } else { i(&v_kind) };
            let Some(v_kind_ok) = v_kind_text.filter(|k| MISSION_KINDS.contains(&k.as_str())) else {
                self.reply(
                    p_conn,
                    p_message,
                    SAVE_MISSION,
                    Ms::new(format!("ERROR:unknown mission kind '{}'", v_kind_display))
                        .o("missionId", &v_mission_id)
                        .o("unitId", &v_unit)
                        .o("rq", &v_rq),
                    false,
                    Some("unknown mission kind"),
                );
                return Ok(());
            };
            if v_empty(&v_mission_id) {
                v_mission_id = s(&uuid_v4());
            }
            let v_id = &v_mission_id;
            let st = self.st();
            st.log.info(&format!(
                "SaveMission request for unit {}, mission {} (kind {})",
                i(&v_unit),
                i(v_id),
                v_kind_ok
            ));
            st.db.upsert_unit(&v_unit, &Some(Value::Null), &v_cs)?;

            let v_limits = match st.cfg().get("limits") {
                Some(v) if js_truthy(Some(v)) => v.clone(),
                _ => json!({}),
            };
            let v_limit = |k: &str, d: Value| field(&v_limits, k).filter(|v| !v.is_null()).cloned().unwrap_or(d);
            let reject = |p_text: String, p_error: &str| {
                self.reply(
                    p_conn,
                    p_message,
                    SAVE_MISSION,
                    Ms::new(p_text).o("missionId", v_id).o("unitId", &v_unit).o("rq", &v_rq),
                    false,
                    Some(p_error),
                );
            };

            let v_max = v_limit("max_mission_bytes", json!(MISSION_DATA_MAX_BYTES_DEFAULT as i64));
            let v_bytes = js_stringify(v_data.as_ref().unwrap_or(&Value::Null)).len() as f64;
            if js_number(Some(&v_max)) > 0.0 && v_bytes > js_number(Some(&v_max)) {
                st.log.warn(&format!(
                    "SaveMission rejected: mission {} is {} bytes (limit {})",
                    i(v_id),
                    c(v_bytes),
                    js_key(Some(&v_max))
                ));
                reject(
                    format!("ERROR:mission too large ({} bytes > {})", c(v_bytes), js_key(Some(&v_max))),
                    "mission too large",
                );
                return Ok(());
            }
            if let Some(m) = v_meta.as_ref().filter(|m| !m.is_null()) {
                if js_stringify(m).len() > MISSION_META_MAX_BYTES {
                    reject(
                        format!("ERROR:mission meta too large (> {} bytes)", MISSION_META_MAX_BYTES),
                        "mission meta too large",
                    );
                    return Ok(());
                }
            }

            let v_owner = st.db.get_mission_account_id(v_id)?.filter(|o| !o.is_null());
            if let Some(o) = &v_owner {
                if !js_strict_eq(Some(o), v_account.as_ref()) {
                    st.log.warn(&format!(
                        "SaveMission rejected: id {} belongs to another account (caller {})",
                        i(v_id),
                        i(&v_account)
                    ));
                    reject(
                        "ERROR:mission id belongs to another account".into(),
                        "mission id belongs to another account",
                    );
                    return Ok(());
                }
            }

            if v_owner.is_none() {
                let v_per_account = v_limit("max_missions_per_account", json!(0));
                let v_per_unit = v_limit("max_missions_per_unit", json!(0));
                let fv = |v: &Value| js_number(Some(v));
                if fv(&v_per_account) > 0.0 && st.db.count_missions_by_account(&v_account)? >= fv(&v_per_account) {
                    let v_count = st.db.count_missions_by_account(&v_account)?;
                    let v_lim = js_key(Some(&v_per_account));
                    st.log.warn(&format!(
                        "SaveMission rejected: account {} at {}/{}",
                        i(&v_account),
                        c(v_count),
                        v_lim
                    ));
                    reject(
                        format!("ERROR:mission quota exceeded ({}/{} per account)", c(v_count), v_lim),
                        "mission quota exceeded",
                    );
                    return Ok(());
                }
                if fv(&v_per_unit) > 0.0 && st.db.count_missions_by_unit(&v_unit, &v_account)? >= fv(&v_per_unit) {
                    let v_count = st.db.count_missions_by_unit(&v_unit, &v_account)?;
                    let v_lim = js_key(Some(&v_per_unit));
                    st.log.warn(&format!(
                        "SaveMission rejected: account {} unit {} at {}/{}",
                        i(&v_account),
                        i(&v_unit),
                        c(v_count),
                        v_lim
                    ));
                    reject(
                        format!("ERROR:mission quota exceeded ({}/{} per unit)", c(v_count), v_lim),
                        "mission quota exceeded",
                    );
                    return Ok(());
                }
                // (limits.max_missions_per_kind && limits.max_missions_per_kind[kind]) ?? 0
                let v_per_kind = match field(&v_limits, "max_missions_per_kind") {
                    Some(k) if js_truthy(Some(k)) => {
                        field(k, &v_kind_ok).filter(|v| !v.is_null()).cloned().unwrap_or(json!(0))
                    }
                    Some(k) if !k.is_null() => k.clone(),
                    _ => json!(0),
                };
                if fv(&v_per_kind) > 0.0 && st.db.count_missions_by_kind(&v_account, &v_kind_ok)? >= fv(&v_per_kind) {
                    let v_count = st.db.count_missions_by_kind(&v_account, &v_kind_ok)?;
                    let v_lim = js_key(Some(&v_per_kind));
                    st.log.warn(&format!(
                        "SaveMission rejected: account {} kind {} at {}/{}",
                        i(&v_account),
                        v_kind_ok,
                        c(v_count),
                        v_lim
                    ));
                    reject(
                        format!("ERROR:mission quota exceeded ({}/{} per kind {})", c(v_count), v_lim, v_kind_ok),
                        "mission quota exceeded",
                    );
                    return Ok(());
                }
            }

            st.db.save_mission(v_id, &v_unit, &v_account, &v_name, &v_data, &v_kind_ok, &v_meta)?;
            st.db.log_access(&v_unit, "save", "mission", v_id, &v_cs)?;
            let v_saved = st.db.get_mission(v_id, &v_account)?;
            let v_version = match &v_saved {
                Some(r) => r.get("version").cloned(),
                None => Some(json!(1)),
            };
            self.reply(
                p_conn,
                p_message,
                SAVE_MISSION,
                Ms::new("OK:save")
                    .o("missionId", v_id)
                    .o("unitId", &v_unit)
                    .k("kind", json!(v_kind_ok))
                    .o("version", &v_version)
                    .o("rq", &v_rq),
                true,
                None,
            );
            st.log.info(&format!("Saved mission {} for unit {}", i(v_id), i(&v_unit)));
            Ok(())
        })();
        if let Err(e) = v_res {
            self.st().log.error(&format!("Error saving mission for unit {}: {}", i(&v_unit), e));
            self.fail(p_conn, p_message, SAVE_MISSION, &e, |m| {
                m.o("missionId", &v_mission_id).o("unitId", &v_unit).o("rq", &v_rq)
            });
        }
    }

    fn delete_mission(&self, p_conn: u64, p_message: &Value) {
        let v_payload = payload(p_message);
        let v_mission = get(v_payload, "missionId");
        let v_account = get(v_payload, "accountId");
        let v_res = (|| -> Result<(), String> {
            self.st().log.info(&format!("DeleteMission request for mission {}", i(&v_mission)));
            let v_result = self.st().db.delete_mission(&v_mission, &v_account)?;
            self.reply(
                p_conn,
                p_message,
                DELETE_MISSION,
                Ms::new("OK:delete").o("missionId", &v_mission).k("deleted", json!(v_result.changes)),
                true,
                None,
            );
            self.st().log.info(&format!("Deleted mission {} (changes: {})", i(&v_mission), v_result.changes));
            Ok(())
        })();
        if let Err(e) = v_res {
            self.st().log.error(&format!("Error deleting mission {}: {}", i(&v_mission), e));
            self.fail(p_conn, p_message, DELETE_MISSION, &e, |m| m.o("missionId", &v_mission));
        }
    }

    fn load_news(&self, p_conn: u64, p_message: &Value) {
        let v_account = get(payload(p_message), "accountId");
        let v_res = (|| -> Result<(), String> {
            self.st().log.debug(&format!("LoadNews request for account {}", i(&v_account)));
            let v_news = self.st().db.load_news(&v_account)?;
            let v_count = v_news.len();
            self.reply(p_conn, p_message, LOAD_NEWS, Ms::new("OK").k("news", Value::Array(v_news)), true, None);
            self.st().log.debug(&format!("Loaded {} news items for account {}", v_count, i(&v_account)));
            Ok(())
        })();
        if let Err(e) = v_res {
            self.st().log.error(&format!("Error loading news for account {}: {}", i(&v_account), e));
            self.fail(p_conn, p_message, LOAD_NEWS, &e, |m| m);
        }
    }

    fn save_news(&self, p_conn: u64, p_message: &Value) {
        let v_payload = payload(p_message);
        let [v_news_id, v_scope, v_account, v_title, v_body, v_priority, v_author, v_expires] =
            ["newsId", "scope", "accountId", "title", "body", "priority", "authorId", "expiresAt"]
                .map(|k| get(v_payload, k));
        let v_res = (|| -> Result<(), String> {
            let st = self.st();
            let v_id: Arg = if js_truthy(v_news_id.as_ref()) { v_news_id.clone() } else { s(&uuid_v4()) };
            let v_account_text = if js_truthy(v_account.as_ref()) { i(&v_account) } else { "null".into() };
            st.log.info(&format!("SaveNews request {} (scope={}, account={})", i(&v_id), i(&v_scope), v_account_text));
            let v_existing = st.db.get_news(&v_id)?;
            if v_existing.is_none()
                && js_strict_eq(v_scope.as_ref(), Some(&json!("account")))
                && js_truthy(v_account.as_ref())
            {
                let v_limit = st
                    .cfg()
                    .get("limits")
                    .and_then(|l| field(l, "max_news_per_account"))
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or(json!(0));
                if js_number(Some(&v_limit)) > 0.0 {
                    let v_count = st.db.count_news_by_account(&v_account)?;
                    if v_count >= js_number(Some(&v_limit)) {
                        let v_lim = js_key(Some(&v_limit));
                        st.log.warn(&format!(
                            "SaveNews rejected: account {} at {}/{}",
                            i(&v_account),
                            c(v_count),
                            v_lim
                        ));
                        self.reply(
                            p_conn,
                            p_message,
                            SAVE_NEWS,
                            Ms::new(format!("ERROR:news quota exceeded ({}/{})", c(v_count), v_lim)).o("newsId", &v_id),
                            false,
                            Some("news quota exceeded"),
                        );
                        return Ok(());
                    }
                }
            }
            st.db.save_news(&v_id, &v_scope, &v_account, &v_title, &v_body, &v_priority, &v_author, &v_expires)?;
            let v_saved = st.db.get_news(&v_id)?;
            self.reply(p_conn, p_message, SAVE_NEWS, Ms::new("OK:save").o("newsId", &v_id), true, None);
            st.broadcast(&envelope(p_message, NEWS_PUSH, Ms::empty().o("news", &v_saved).done(), true, None));
            st.log.info(&format!("Saved news {}", i(&v_id)));
            Ok(())
        })();
        if let Err(e) = v_res {
            self.st().log.error(&format!("Error saving news: {}", e));
            self.fail(p_conn, p_message, SAVE_NEWS, &e, |m| m);
        }
    }

    fn delete_news(&self, p_conn: u64, p_message: &Value) {
        let v_news_id = get(payload(p_message), "newsId");
        let v_res = (|| -> Result<(), String> {
            let st = self.st();
            st.log.info(&format!("DeleteNews request for news {}", i(&v_news_id)));
            let v_result = st.db.disable_news(&v_news_id)?;
            self.reply(
                p_conn,
                p_message,
                DELETE_NEWS,
                Ms::new("OK:delete").o("newsId", &v_news_id).k("deleted", json!(v_result.changes)),
                true,
                None,
            );
            let v_news = Ms::empty().o("id", &v_news_id).k("disabled", json!(1)).done();
            st.broadcast(&envelope(p_message, NEWS_PUSH, json!({ "news": v_news }), true, None));
            st.log.info(&format!("Disabled news {} (changes: {})", i(&v_news_id), v_result.changes));
            Ok(())
        })();
        if let Err(e) = v_res {
            self.st().log.error(&format!("Error deleting news {}: {}", i(&v_news_id), e));
            self.fail(p_conn, p_message, DELETE_NEWS, &e, |m| m.o("newsId", &v_news_id));
        }
    }
}

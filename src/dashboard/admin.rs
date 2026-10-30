//! The /admin router (js_router_admin.js): session, CSP, the optional GUID
//! gate, CSRF, login (bcrypt or plaintext, rate limited, with lockout),
//! the pages, and the JSON API over the database.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;

use super::http::{Request, Response};
use super::limiter::{seconds_left, Limiter};
use super::session::{self, Session, Store};
use super::{csrf, App};
use crate::colors as c;
use crate::db::{n, s, Arg};
use crate::util::{field, js_interp, js_parse_int, js_strict_eq, js_truthy, now_ms};

const LOCKOUT_MS: i64 = 30 * 60 * 1000;
const FAILED_ATTEMPTS_MAX_ENTRIES: usize = 10000;

/// Account lockout: failed logins per `ip:username`.
#[derive(Default)]
pub struct Lockout {
    m_failed: Mutex<HashMap<String, (u32, i64)>>,
}

impl Lockout {
    fn locked(&self, p_key: &str) -> bool {
        let mut m = self.m_failed.lock().unwrap();
        let Some((v_count, v_last)) = m.get(p_key).copied() else { return false };
        if now_ms() - v_last > LOCKOUT_MS {
            m.remove(p_key);
            return false;
        }
        v_count >= 5
    }

    fn record(&self, p_key: &str) {
        let mut m = self.m_failed.lock().unwrap();
        let e = m.entry(p_key.to_string()).or_insert((0, 0));
        e.0 += 1;
        e.1 = now_ms();
        if m.len() > FAILED_ATTEMPTS_MAX_ENTRIES {
            let mut v_all: Vec<(String, i64)> = m.iter().map(|(k, v)| (k.clone(), v.1)).collect();
            v_all.sort_by_key(|(_, t)| *t);
            let v_excess = m.len() - FAILED_ATTEMPTS_MAX_ENTRIES;
            for (k, _) in v_all.into_iter().take(v_excess) {
                m.remove(&k);
            }
        }
    }

    fn clear(&self, p_key: &str) {
        self.m_failed.lock().unwrap().remove(p_key);
    }

    /// The 5-minute sweep of stale entries.
    pub fn sweep(&self) {
        let v_now = now_ms();
        self.m_failed.lock().unwrap().retain(|_, (_, t)| v_now - *t <= LOCKOUT_MS);
    }
}

pub struct AdminRouter {
    pub sessions: Store,
    pub login_limiter: Limiter,
    pub api_limiter: Limiter,
    pub lockout: Lockout,
    /// session_secret, read once like the session middleware
    pub secret: Option<String>,
    /// cookie.secure (enable_SSL at load)
    pub secure_cookie: bool,
}

impl AdminRouter {
    pub fn new(p_secret: Option<String>, p_secure_cookie: bool) -> AdminRouter {
        AdminRouter {
            sessions: Store::default(),
            login_limiter: Limiter::new(15 * 60 * 1000, 5),
            api_limiter: Limiter::new(60 * 1000, 60),
            lockout: Lockout::default(),
            secret: p_secret,
            secure_cookie: p_secure_cookie,
        }
    }
}

/// adminPath(p): the admin URL with the GUID prefix when configured.
pub fn admin_path(p_app: &App, p_path: &str) -> String {
    match guid(p_app) {
        Some(g) => format!("/admin/{}{}", g, p_path),
        None => format!("/admin{}", p_path),
    }
}

fn guid(p_app: &App) -> Option<String> {
    let v_cfg = p_app.storage.cfg();
    v_cfg.get("dashboard_url_guid").filter(|g| js_truthy(Some(g))).map(|g| js_interp(Some(g)))
}

/// An Express route path match: case-insensitive, optional trailing slash.
fn is(p_path: &str, p_route: &str) -> bool {
    let v = if p_path.len() > 1 { p_path.strip_suffix('/').unwrap_or(p_path) } else { p_path };
    v.eq_ignore_ascii_case(p_route)
}

/// `/api/news/:id` style match: the decoded parameter.
fn param(p_path: &str, p_prefix: &str, p_suffix: &str) -> Option<String> {
    let v = if p_path.len() > 1 { p_path.strip_suffix('/').unwrap_or(p_path) } else { p_path };
    if v.len() < p_prefix.len() || !v[..p_prefix.len()].eq_ignore_ascii_case(p_prefix) {
        return None;
    }
    let v_rest = &v[p_prefix.len()..];
    let v_id = if p_suffix.is_empty() {
        v_rest
    } else {
        let i = v_rest.len().checked_sub(p_suffix.len())?;
        if !v_rest[i..].eq_ignore_ascii_case(p_suffix) {
            return None;
        }
        &v_rest[..i]
    };
    if v_id.is_empty() || v_id.contains('/') {
        return None;
    }
    super::http::percent_decode(v_id)
}

/// `/api` mount (router.use('/api', ...)).
fn under_api(p_path: &str) -> bool {
    p_path.len() >= 4 && p_path[..4].eq_ignore_ascii_case("/api") && (p_path.len() == 4 || p_path.as_bytes()[4] == b'/')
}

/// The JavaScript error class the console would print for a database error.
fn error_name(p_message: &str) -> &'static str {
    if p_message.starts_with("Too many parameter") || p_message.starts_with("Too few parameter") {
        "RangeError"
    } else if p_message.starts_with("SQLite3 can only bind")
        || p_message.starts_with("You cannot specify named")
        || p_message.contains("is not a function")
    {
        "TypeError"
    } else {
        "SqliteError"
    }
}

struct Ctx<'a> {
    app: &'a App,
    req: &'a Request,
    body: &'a Value,
    query: Value,
    session: Session,
    res: Response,
    csrf_secret: String,
}

impl Ctx<'_> {
    fn q(&self, p_key: &str) -> Arg {
        field(&self.query, p_key).cloned()
    }

    /// `parseInt(req.query.x) || d`
    fn qint(&self, p_key: &str, p_default: f64) -> f64 {
        match js_parse_int(self.q(p_key).as_ref()) {
            Some(i) if i != 0 => i as f64,
            _ => p_default,
        }
    }

    /// `req.query.x || null`
    fn qor(&self, p_key: &str) -> Arg {
        match self.q(p_key) {
            Some(v) if js_truthy(Some(&v)) => Some(v),
            _ => Some(Value::Null),
        }
    }

    /// `(req.query.x || '').trim() || null`; Err for a value without trim().
    fn qtrim(&self, p_key: &str) -> Result<Arg, String> {
        match self.q(p_key) {
            Some(Value::String(t)) => {
                let t = t.trim();
                Ok(Some(if t.is_empty() { Value::Null } else { json!(t) }))
            }
            Some(v) if js_truthy(Some(&v)) => Err(format!("(req.query.{} || \"\").trim is not a function", p_key)),
            _ => Ok(Some(Value::Null)),
        }
    }

    fn username(&self) -> String {
        js_interp(self.session.get("adminUsername"))
    }

    fn render(&mut self, p_view: &str, p_ctx: minijinja::Value) {
        match self.app.render(p_view, p_ctx) {
            Ok(html) => self.res.html(self.req, html),
            Err(e) => {
                eprintln!("Error: {}", e);
                self.res = Response::error_page(500, "Error", &e);
            }
        }
    }

    fn page_ctx(&self, p_title: &str) -> minijinja::Value {
        let v_guid = guid(self.app);
        minijinja::context! {
            title => p_title,
            adminUsername => self.session.get("adminUsername").map(|v| js_interp(Some(v))),
            dashboardGuid => v_guid,
            csrfToken => csrf::create(&self.csrf_secret),
        }
    }

    fn redirect(&mut self, p_path: &str) {
        let v_url = admin_path(self.app, p_path);
        self.res.redirect(self.req, &v_url);
    }

    fn json(&mut self, p_value: Value) {
        self.res.json(self.req, &p_value);
    }

    /// The catch blocks of the API routes.
    fn api_error(&mut self, p_what: &str, p_reply: &str, p_err: &str) {
        eprintln!("Error {}: {}: {}", p_what, error_name(p_err), p_err);
        self.json(json!({ "error": 1, "errorMessage": p_reply }));
    }
}

/// The router for a request whose path is under /admin; p_path is the rest
/// (req.path inside the router). None falls through to the app's 404.
pub fn handle(p_app: &App, p_req: &Request, p_body: &Value, p_path: &str) -> Option<Response> {
    let v_router = &p_app.admin;
    let Some(v_secret) = v_router.secret.clone() else {
        eprintln!("Error: secret option required for sessions");
        return Some(Response::error_page(500, "Error", "secret option required for sessions"));
    };
    let v_session = v_router.sessions.load(p_req.cookie(session::COOKIE_NAME).as_deref(), &v_secret);
    let mut ctx = Ctx {
        app: p_app,
        req: p_req,
        body: p_body,
        query: p_req.query(),
        session: v_session,
        res: Response::new(),
        csrf_secret: String::new(),
    };
    let v_handled = route(&mut ctx, p_path);
    if !v_handled {
        // fell out of the router: the 404 page still carries the router's headers
        let v_cookie = v_router.sessions.commit(&ctx.session, &v_secret, v_router.secure_cookie, p_req.secure);
        let mut r = ctx.res;
        super::not_found(p_app, p_req, &mut r);
        if let Some(c) = v_cookie {
            r.append("Set-Cookie", c);
        }
        return Some(r);
    }
    let v_cookie = v_router.sessions.commit(&ctx.session, &v_secret, v_router.secure_cookie, p_req.secure);
    let mut r = ctx.res;
    if let Some(c) = v_cookie {
        r.append("Set-Cookie", c);
    }
    Some(r)
}

/// The middleware and routes in order; false when nothing answered.
fn route(ctx: &mut Ctx<'_>, p_path: &str) -> bool {
    let mut v_path = p_path.to_string();
    // GUID gate
    if let Some(g) = guid(ctx.app) {
        let v_prefix = format!("/{}", g);
        if v_path == v_prefix || v_path.starts_with(&format!("{}/", v_prefix)) {
            let v_url = format!(
                "{}{}",
                &v_path[v_prefix.len()..],
                if ctx.req.query_raw.is_empty() { String::new() } else { format!("?{}", ctx.req.query_raw) }
            );
            if v_url.is_empty() || v_url == "/" {
                ctx.redirect("/login");
                return true;
            }
            v_path = v_path[v_prefix.len()..].to_string();
        } else if !ctx.session.authenticated() {
            ctx.res.status = 404;
            let v_page =
                ctx.app.render("pages/404.html", minijinja::context! { title => "404", message => "Not found." });
            match v_page {
                Ok(h) => ctx.res.html(ctx.req, h),
                Err(e) => ctx.res = Response::error_page(500, "Error", &e),
            }
            return true;
        }
    }

    // CSRF: the secret cookie, then the token check for writes
    ctx.csrf_secret = match ctx.req.cookie(csrf::COOKIE_NAME) {
        Some(v) if !v.is_empty() => v,
        _ => {
            let v = csrf::new_secret();
            ctx.res
                .append("Set-Cookie", format!("{}={}; Path=/", csrf::COOKIE_NAME, super::http::encode_component(&v)));
            v
        }
    };
    let v_method = if ctx.req.method == "HEAD" { "GET".to_string() } else { ctx.req.method.clone() };
    if !matches!(v_method.as_str(), "GET" | "OPTIONS") {
        let v_token = [
            field(ctx.body, "_csrf").cloned(),
            field(&ctx.query, "_csrf").cloned(),
            ctx.req.header("csrf-token").map(|h| json!(h)),
            ctx.req.header("xsrf-token").map(|h| json!(h)),
            ctx.req.header("x-csrf-token").map(|h| json!(h)),
            ctx.req.header("x-xsrf-token").map(|h| json!(h)),
        ]
        .into_iter()
        .flatten()
        .find(|v| js_truthy(Some(v)));
        if !csrf::verify(&ctx.csrf_secret, v_token.as_ref().and_then(Value::as_str)) {
            if v_path.starts_with("/api/") {
                ctx.res.status = 403;
                ctx.json(json!({ "error": 1, "errorMessage": "Invalid or missing CSRF token" }));
            } else {
                ctx.session.set("error", json!("Session expired, please log in again"));
                ctx.redirect("/login");
            }
            return true;
        }
    }

    let p = v_path.as_str();
    match v_method.as_str() {
        "GET" if is(p, "/login") => {
            let v_error = ctx.session.get("error").cloned();
            let v_admin = admin_path(ctx.app, "");
            let v_page = minijinja::context! {
                csrfToken => csrf::create(&ctx.csrf_secret),
                error => v_error.filter(|e| js_truthy(Some(e))).map(|e| js_interp(Some(&e))),
                title => "Storage Admin Login",
                adminPath => v_admin,
            };
            // login.js then sets session.error = null, but only after the
            // (synchronous) render already saved the session: the message stays
            ctx.render("admin/login.html", v_page);
            return true;
        }
        "POST" if is(p, "/login") => {
            let v_hit = ctx.app.admin.login_limiter.hit(&ctx.req.ip);
            let v_limiter = &ctx.app.admin.login_limiter;
            ctx.res.set("X-RateLimit-Limit", v_limiter.limit.to_string());
            ctx.res.set("X-RateLimit-Remaining", v_limiter.remaining(&v_hit).to_string());
            ctx.res.set("X-RateLimit-Reset", ((v_hit.reset_ms + 999) / 1000).to_string());
            if v_hit.limited {
                ctx.res.status = 429;
                ctx.res.set("Retry-After", seconds_left(v_hit.reset_ms).to_string());
                ctx.json(json!({ "error": "Too many login attempts, please try again later" }));
                return true;
            }
            return login(ctx);
        }
        "POST" if is(p, "/logout") => {
            ctx.app.admin.sessions.destroy(&mut ctx.session);
            ctx.redirect("/login");
            return true;
        }
        "GET" => {
            let v_pages = [
                ("/dashboard", "admin/dashboard.html", "Storage Dashboard"),
                ("/messages", "admin/messages.html", "Access Log (Messages)"),
                ("/units", "admin/units.html", "Units"),
                ("/tasks", "admin/tasks.html", "Tasks"),
                ("/missions", "admin/missions.html", "Missions"),
                ("/news", "admin/news.html", "News"),
                ("/queue", "admin/queue.html", "Offline Queue"),
                ("/settings", "admin/settings.html", "Settings"),
            ];
            if let Some((v_route, v_view, v_title)) = v_pages.iter().find(|(r, _, _)| is(p, r)) {
                if !ctx.session.authenticated() {
                    ctx.redirect("/login");
                    return true;
                }
                let mut v_ctx = ctx.page_ctx(v_title);
                if *v_route == "/dashboard" {
                    let v_cfg = ctx.app.storage.cfg();
                    let v_server_id = v_cfg.get("server_id").map(|v| js_interp(Some(v)));
                    let v_db_path = v_cfg.get("database").and_then(|d| d.get("path")).map(|v| js_interp(Some(v)));
                    drop(v_cfg);
                    v_ctx = minijinja::context! { serverId => v_server_id, dbPath => v_db_path, ..v_ctx };
                }
                ctx.render(v_view, v_ctx);
                return true;
            }
        }
        _ => {}
    }

    if !under_api(p) {
        return false;
    }
    // router.use('/api', apiLimiter)
    let v_hit = ctx.app.admin.api_limiter.hit(&ctx.req.ip);
    let v_limiter = &ctx.app.admin.api_limiter;
    ctx.res.set("RateLimit-Policy", format!("{};w={}", v_limiter.limit, v_limiter.window_ms / 1000));
    ctx.res.set("RateLimit-Limit", v_limiter.limit.to_string());
    ctx.res.set("RateLimit-Remaining", v_limiter.remaining(&v_hit).to_string());
    ctx.res.set("RateLimit-Reset", seconds_left(v_hit.reset_ms).to_string());
    if v_hit.limited {
        ctx.res.status = 429;
        ctx.res.set("Retry-After", seconds_left(v_hit.reset_ms).to_string());
        ctx.json(json!({ "error": 1, "errorMessage": "Too many API requests, please slow down." }));
        return true;
    }
    api(ctx, p, &v_method)
}

use droneengage_server_common::config_handler::is_bcrypt_hash;
use droneengage_server_common::validation::{
    is_valid_admin_password as valid_password, is_valid_admin_username as valid_username,
};

/// POST /login
fn login(ctx: &mut Ctx<'_>) -> bool {
    let v_user = field(ctx.body, "username").cloned();
    let v_pass = field(ctx.body, "password").cloned();
    let v_ip = ctx.req.ip.clone();
    let v_fail = |ctx: &mut Ctx<'_>, p_msg: &str| {
        ctx.session.set("error", json!(p_msg));
        ctx.redirect("/login");
        true
    };
    match valid_username(v_user.as_ref()) {
        Ok(true) => {}
        Ok(false) => return v_fail(ctx, "Invalid username format"),
        Err(e) => {
            eprintln!("TypeError: {}", e);
            ctx.res = Response::error_page(500, "TypeError", &e);
            return true;
        }
    }
    if !valid_password(v_pass.as_ref()) {
        return v_fail(ctx, "Invalid password format");
    }
    let v_key = format!("{}:{}", v_ip, js_interp(v_user.as_ref()));
    if ctx.app.admin.lockout.locked(&v_key) {
        return v_fail(ctx, "Account temporarily locked due to too many failed attempts. Please try again later.");
    }
    let (v_admin, v_stored) = {
        let v_cfg = ctx.app.storage.cfg();
        (v_cfg.get("admin_username").cloned(), v_cfg.get("admin_password").cloned())
    };
    if !js_strict_eq(v_user.as_ref(), v_admin.as_ref()) {
        ctx.app.admin.lockout.record(&v_key);
        return v_fail(ctx, "Invalid username or password");
    }
    let v_hashed = is_bcrypt_hash(v_stored.as_ref());
    let v_match = if v_hashed {
        match (v_pass.as_ref(), v_stored.as_ref()) {
            (Some(Value::String(p)), Some(Value::String(h))) => h.len() == 60 && bcrypt::verify(p, h).unwrap_or(false),
            (v_p, _) => {
                let v_type = match v_p {
                    Some(Value::Number(_)) => "number",
                    Some(Value::Bool(_)) => "boolean",
                    _ => "object",
                };
                let e = format!("Illegal arguments: {}, string", v_type);
                eprintln!("Error: {}", e);
                ctx.res = Response::error_page(500, "Error", &e);
                return true;
            }
        }
    } else {
        js_strict_eq(v_pass.as_ref(), v_stored.as_ref())
    };
    if !v_match {
        ctx.app.admin.lockout.record(&v_key);
        return v_fail(ctx, "Invalid username or password");
    }
    if !v_hashed {
        eprintln!(
            "{}[WARN] admin_password is stored in plaintext. Use $$HASH$$('...') in server.config and restart to hash it.{}",
            c::FG_YELLOW,
            c::RESET
        );
    }
    ctx.app.admin.lockout.clear(&v_key);
    ctx.app.admin.sessions.regenerate(&mut ctx.session);
    ctx.session.set("adminAuthenticated", json!(true));
    if let Some(u) = v_user {
        ctx.session.set("adminUsername", u);
    }
    ctx.redirect("/dashboard");
    true
}

/// broadcastNewsPush
fn news_push(p_app: &App, p_news: Value) {
    p_app.storage.broadcast(&json!({ "mt": 9018, "ms": { "news": p_news }, "success": true, "timestamp": now_ms() }));
}

/// The API routes (after the limiter); false when none matches.
fn api(ctx: &mut Ctx<'_>, p: &str, p_method: &str) -> bool {
    let v_db = &ctx.app.storage.db;
    let v_auth = ctx.session.authenticated();
    macro_rules! auth {
        () => {
            if !v_auth {
                ctx.redirect("/login");
                return true;
            }
        };
    }
    let page = |r: Result<crate::db::Page, String>| -> Result<Value, String> {
        let mut m = Map::new();
        m.insert("error".into(), json!(0));
        r?.into_fields(&mut m);
        Ok(Value::Object(m))
    };
    match p_method {
        "GET" if is(p, "/api/stats") => {
            auth!();
            let v_res = (|| -> Result<Value, String> {
                let v_stats = v_db.get_stats()?;
                let v_queue = v_db.get_queue_stats()?;
                let mut m = v_stats.as_object().cloned().unwrap_or_default();
                let v_size = m.get("dbSize").and_then(Value::as_f64).unwrap_or(0.0);
                m.insert("pendingMessages".into(), v_queue.get("pendingMessages").cloned().unwrap_or(Value::Null));
                m.insert("affectedUnits".into(), v_queue.get("affectedUnits").cloned().unwrap_or(Value::Null));
                m.insert("connections".into(), json!(ctx.app.storage.connection_count()));
                m.insert("dbSizeMB".into(), json!(format!("{:.2}", v_size / 1024.0 / 1024.0)));
                Ok(json!({ "error": 0, "stats": m }))
            })();
            match v_res {
                Ok(v) => ctx.json(v),
                Err(e) => ctx.api_error("fetching stats", "Failed to fetch stats", &e),
            }
        }
        "GET" if is(p, "/api/messages") => {
            auth!();
            let r = v_db.get_access_log_page(
                ctx.qint("page", 1.0),
                ctx.qint("limit", 50.0),
                &ctx.qor("action"),
                &ctx.qor("unitId"),
            );
            match page(r) {
                Ok(v) => ctx.json(v),
                Err(e) => ctx.api_error("fetching messages", "Failed to fetch messages", &e),
            }
        }
        "DELETE" if is(p, "/api/messages") => {
            auth!();
            let v_res = (|| -> Result<u64, String> {
                let (a, u) = (ctx.qtrim("action")?, ctx.qtrim("unitId")?);
                let v_deleted = v_db.clear_access_log(&a, &u)?;
                let f = |k: &str, v: &Arg| {
                    if js_truthy(v.as_ref()) {
                        format!(" ({}={})", k, js_interp(v.as_ref()))
                    } else {
                        String::new()
                    }
                };
                println!(
                    "[dashboard] access_log cleared by {}: {} rows removed{}{}",
                    ctx.username(),
                    v_deleted,
                    f("action", &a),
                    f("unitId", &u)
                );
                Ok(v_deleted)
            })();
            match v_res {
                Ok(d) => ctx.json(json!({ "error": 0, "deleted": d })),
                Err(e) => ctx.api_error("clearing messages", "Failed to clear messages", &e),
            }
        }
        "GET" if is(p, "/api/units") => {
            auth!();
            let r = ctx
                .qtrim("search")
                .and_then(|sv| v_db.get_units_page(ctx.qint("page", 1.0), ctx.qint("limit", 50.0), &sv));
            match page(r) {
                Ok(v) => ctx.json(v),
                Err(e) => ctx.api_error("fetching units", "Failed to fetch units", &e),
            }
        }
        "GET" if is(p, "/api/tasks") => {
            auth!();
            let v_include = !js_strict_eq(ctx.q("includeDisabled").as_ref(), Some(&json!("false")));
            let r = v_db.get_tasks_page(ctx.qint("page", 1.0), ctx.qint("limit", 50.0), &ctx.qor("unitId"), v_include);
            match page(r) {
                Ok(v) => ctx.json(v),
                Err(e) => ctx.api_error("fetching tasks", "Failed to fetch tasks", &e),
            }
        }
        "GET" if is(p, "/api/missions") => {
            auth!();
            let r = v_db.get_missions_page(
                ctx.qint("page", 1.0),
                ctx.qint("limit", 50.0),
                &ctx.qor("unitId"),
                &ctx.qor("accountId"),
                &ctx.qor("kind"),
            );
            match page(r) {
                Ok(v) => ctx.json(v),
                Err(e) => ctx.api_error("fetching missions", "Failed to fetch missions", &e),
            }
        }
        "GET" if is(p, "/api/queue") => {
            auth!();
            let r = v_db.get_queue_page(
                ctx.qint("page", 1.0),
                ctx.qint("limit", 50.0),
                &ctx.qor("unitId"),
                &ctx.qor("status"),
            );
            match page(r) {
                Ok(v) => ctx.json(v),
                Err(e) => ctx.api_error("fetching queue", "Failed to fetch queue", &e),
            }
        }
        "GET" if is(p, "/api/units/list") => {
            auth!();
            match v_db.get_all_unit_ids() {
                Ok(v) => ctx.json(json!({ "error": 0, "unitIds": v })),
                Err(e) => ctx.api_error("fetching unit list", "Failed to fetch unit list", &e),
            }
        }
        "GET" if is(p, "/api/missions/accounts") => {
            auth!();
            match v_db.get_mission_accounts() {
                Ok(v) => ctx.json(json!({ "error": 0, "accounts": v })),
                Err(e) => ctx.api_error("fetching accounts", "Failed to fetch accounts", &e),
            }
        }
        "GET" if is(p, "/api/news") => {
            auth!();
            let v_include = !js_strict_eq(ctx.q("includeDisabled").as_ref(), Some(&json!("false")));
            let r = v_db.get_news_page(
                ctx.qint("page", 1.0),
                ctx.qint("limit", 50.0),
                &ctx.qor("scope"),
                &ctx.qor("accountId"),
                v_include,
            );
            match page(r) {
                Ok(v) => ctx.json(v),
                Err(e) => ctx.api_error("fetching news", "Failed to fetch news", &e),
            }
        }
        "GET" if is(p, "/api/news/accounts") => {
            auth!();
            match v_db.get_news_accounts() {
                Ok(v) => ctx.json(json!({ "error": 0, "accounts": v })),
                Err(e) => ctx.api_error("fetching news accounts", "Failed to fetch news accounts", &e),
            }
        }
        "POST" if is(p, "/api/news") => {
            auth!();
            let b = |k: &str| field(ctx.body, k).cloned();
            let (v_scope, v_account, v_title, v_body, v_priority, v_expires) =
                (b("scope"), b("accountId"), b("title"), b("body"), b("priority"), b("expiresAt"));
            let v_global = js_strict_eq(v_scope.as_ref(), Some(&json!("global")));
            if !v_global && !js_strict_eq(v_scope.as_ref(), Some(&json!("account"))) {
                ctx.json(json!({ "error": 1, "errorMessage": "scope must be 'global' or 'account'" }));
                return true;
            }
            if !v_global && !js_truthy(v_account.as_ref()) {
                ctx.json(json!({ "error": 1, "errorMessage": "accountId is required for scope=account" }));
                return true;
            }
            if !js_truthy(v_body.as_ref()) {
                ctx.json(json!({ "error": 1, "errorMessage": "body is required" }));
                return true;
            }
            let v_res = (|| -> Result<Arg, String> {
                let v_id: Arg = match b("newsId") {
                    Some(v) if js_truthy(Some(&v)) => Some(v),
                    _ => s(&crate::util::uuid_v4()),
                };
                let v_title2 = if js_truthy(v_title.as_ref()) { v_title.clone() } else { Some(Value::Null) };
                let v_prio = n(js_parse_int(v_priority.as_ref()).filter(|p| *p != 0).unwrap_or(0) as f64);
                let v_exp = if js_truthy(v_expires.as_ref()) { v_expires.clone() } else { Some(Value::Null) };
                let v_acc = if v_global { Some(Value::Null) } else { v_account.clone() };
                let v_user = ctx.session.get("adminUsername").cloned();
                v_db.save_news(&v_id, &v_scope, &v_acc, &v_title2, &v_body, &v_prio, &v_user, &v_exp)?;
                let v_saved = v_db.get_news(&v_id)?;
                println!(
                    "[dashboard] news {} saved by {} (scope={}, expiresAt={})",
                    js_interp(v_id.as_ref()),
                    ctx.username(),
                    js_interp(v_scope.as_ref()),
                    js_interp(v_expires.as_ref())
                );
                news_push(ctx.app, v_saved.unwrap_or_else(crate::util::undefined));
                Ok(v_id)
            })();
            match v_res {
                Ok(id) => {
                    let mut m = Map::new();
                    m.insert("error".into(), json!(0));
                    if let Some(id) = id {
                        m.insert("newsId".into(), id);
                    }
                    ctx.json(Value::Object(m))
                }
                Err(e) => ctx.api_error("saving news", "Failed to save news", &e),
            }
        }
        "POST" if param(p, "/api/news/", "/enable").is_some() => {
            auth!();
            let v_id = s(&param(p, "/api/news/", "/enable").unwrap());
            let v_res = (|| -> Result<u64, String> {
                let r = v_db.enable_news(&v_id)?;
                let v_saved = v_db.get_news(&v_id)?;
                println!("[dashboard] news {} enabled by {}", js_interp(v_id.as_ref()), ctx.username());
                if let Some(n) = v_saved {
                    news_push(ctx.app, n);
                }
                Ok(r.changes)
            })();
            match v_res {
                Ok(c) => ctx.json(json!({ "error": 0, "enabled": c })),
                Err(e) => ctx.api_error("enabling news", "Failed to enable news", &e),
            }
        }
        "DELETE" if param(p, "/api/news/", "").is_some() => {
            auth!();
            let v_id = s(&param(p, "/api/news/", "").unwrap());
            let v_perm = ctx.q("permanent").map(|v| v == json!("true") || v == json!("1")).unwrap_or(false);
            let v_res = (|| -> Result<u64, String> {
                let v_existing = v_db.get_news(&v_id)?;
                let r = if v_perm { v_db.delete_news(&v_id)? } else { v_db.disable_news(&v_id)? };
                println!(
                    "[dashboard] news {} {} by {}",
                    js_interp(v_id.as_ref()),
                    if v_perm { "deleted" } else { "disabled" },
                    ctx.username()
                );
                let g = |k: &str| v_existing.as_ref().and_then(|e| e.get(k).cloned()).unwrap_or(Value::Null);
                news_push(
                    ctx.app,
                    json!({ "id": v_id, "scope": g("scope"), "account_id": g("account_id"), "disabled": 1 }),
                );
                Ok(r.changes)
            })();
            match v_res {
                Ok(c) => ctx.json(json!({ "error": 0, "disabled": c, "permanent": v_perm })),
                Err(e) => ctx.api_error("disabling news", "Failed to disable news", &e),
            }
        }
        "DELETE" if param(p, "/api/missions/", "").is_some() => {
            auth!();
            let v_id = param(p, "/api/missions/", "").unwrap();
            match v_db.delete_mission_by_id(&s(&v_id)) {
                Ok(r) => {
                    println!("[dashboard] mission {} deleted by {}", v_id, ctx.username());
                    ctx.json(json!({ "error": 0, "deleted": r.changes }))
                }
                Err(e) => ctx.api_error("deleting mission", "Failed to delete mission", &e),
            }
        }
        "GET" if is(p, "/api/settings") => {
            auth!();
            let v_limits =
                ctx.app.storage.cfg().get("limits").filter(|l| js_truthy(Some(l))).cloned().unwrap_or(json!({}));
            let g = |k: &str| field(&v_limits, k).filter(|v| !v.is_null()).cloned().unwrap_or(json!(0));
            let v_persisted = ctx.app.local_config_path().exists();
            ctx.json(json!({
                "error": 0,
                "limits": {
                    "max_news_per_account": g("max_news_per_account"),
                    "max_missions_per_account": g("max_missions_per_account"),
                    "max_missions_per_unit": g("max_missions_per_unit"),
                },
                "persisted": v_persisted,
            }));
        }
        "POST" if is(p, "/api/settings") => {
            auth!();
            let parse = |k: &str| js_parse_int(field(ctx.body, k)).filter(|v| *v >= 0);
            let (Some(v_news), Some(v_missions), Some(v_unit)) =
                (parse("max_news_per_account"), parse("max_missions_per_account"), parse("max_missions_per_unit"))
            else {
                ctx.json(
                    json!({ "error": 1, "errorMessage": "All limits must be non-negative integers (0 = unbounded)" }),
                );
                return true;
            };
            let v_limits = json!({
                "max_news_per_account": v_news,
                "max_missions_per_account": v_missions,
                "max_missions_per_unit": v_unit,
            });
            ctx.app.storage.update_cfg(|c| c.set("limits", v_limits.clone()));
            match ctx.app.write_local_limits(&v_limits) {
                Ok(()) => {
                    println!(
                        "[dashboard] quota limits updated by {}: news={}, missions={}, perUnit={}",
                        ctx.username(),
                        v_news,
                        v_missions,
                        v_unit
                    );
                    ctx.json(json!({ "error": 0, "limits": v_limits }));
                }
                Err(e) => {
                    eprintln!("Error saving settings: Error: {}", e);
                    ctx.json(json!({ "error": 1, "errorMessage": "Failed to save settings" }));
                }
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_bcryptjs_hashes_and_validates_like_server_common() {
        // made by bcryptjs.hashSync('Secret123!', 10)
        let v_hash = "$2b$10$H1pZ7TapumvLDF6itaUnWePipvrpk1WO4zLe3I9.lIyx7Aa9Yq0Ca";
        assert!(is_bcrypt_hash(Some(&json!(v_hash))));
        assert!(bcrypt::verify("Secret123!", v_hash).unwrap());
        assert!(!bcrypt::verify("Secret124!", v_hash).unwrap());
        assert!(!is_bcrypt_hash(Some(&json!("CHANGE_ME"))));
        assert_eq!(valid_username(Some(&json!("admin"))), Ok(true));
        assert_eq!(valid_username(Some(&json!("a.b@c.io"))), Ok(true));
        assert_eq!(valid_username(Some(&json!("ab"))), Ok(false));
        assert_eq!(valid_username(Some(&json!("a!b"))), Ok(false));
        assert_eq!(valid_username(None), Ok(false));
        assert!(valid_username(Some(&json!(12345))).is_err(), "a number reaches str.match");
        assert!(valid_password(Some(&json!("Secret123!"))));
        assert!(!valid_password(Some(&json!("short"))));
        assert!(!valid_password(Some(&json!("has space 123"))));
        assert!(valid_password(Some(&json!(12345678))), "a number has no length and no whitespace");
        assert!(is("/login/", "/login") && is("/LOGIN", "/login") && !is("/login/x", "/login"));
        assert_eq!(param("/api/news/n%201/enable", "/api/news/", "/enable").as_deref(), Some("n 1"));
        assert_eq!(param("/api/news/a/b", "/api/news/", ""), None);
        assert!(under_api("/api") && under_api("/API/x") && !under_api("/apix"));
    }
}

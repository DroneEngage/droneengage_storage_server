//! The admin dashboard over HTTP: login with CSRF, a protected page, the
//! JSON API, settings persisted to <config>.local.

mod common;

use common::*;
use de_storage_server::db::Db;
use de_storage_server::logger::Log;
use de_storage_server::Storage;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

struct Client {
    addr: std::net::SocketAddr,
    jar: HashMap<String, String>,
}

impl Client {
    async fn req(&mut self, p_method: &str, p_path: &str, p_headers: &[(&str, &str)], p_body: &str) -> Resp {
        let mut s = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        let mut h = format!("{} {} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n", p_method, p_path);
        if !self.jar.is_empty() {
            let c: Vec<String> = self.jar.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            h.push_str(&format!("Cookie: {}\r\n", c.join("; ")));
        }
        for (k, v) in p_headers {
            h.push_str(&format!("{}: {}\r\n", k, v));
        }
        h.push_str(&format!("Content-Length: {}\r\n\r\n{}", p_body.len(), p_body));
        s.write_all(h.as_bytes()).await.unwrap();
        let mut v_raw = Vec::new();
        s.read_to_end(&mut v_raw).await.unwrap();
        let t = String::from_utf8_lossy(&v_raw).into_owned();
        let (v_head, v_body) = t.split_once("\r\n\r\n").unwrap();
        let mut v_lines = v_head.lines();
        let v_status = v_lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
        let v_headers: Vec<(String, String)> = v_lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        for (k, v) in &v_headers {
            if k == "set-cookie" {
                let (kv, _) = v.split_once(';').unwrap_or((v, ""));
                let (k, v) = kv.split_once('=').unwrap();
                self.jar.insert(k.to_string(), v.to_string());
            }
        }
        Resp { status: v_status, headers: v_headers, body: v_body.to_string() }
    }
}

impl Resp {
    fn header(&self, p: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == p).map(|(_, v)| v.as_str())
    }

    fn csrf(&self) -> String {
        let i = self.body.find("name=\"_csrf\" value=\"").unwrap() + 20;
        self.body[i..i + self.body[i..].find('"').unwrap()].to_string()
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

#[tokio::test]
async fn login_pages_api_and_settings() {
    let v_dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(concat!(env!("CARGO_MANIFEST_DIR"), "/views"), v_dir.path().join("views")).unwrap();
    std::fs::write(v_dir.path().join("t.config"), "{}").unwrap();
    // dashboard_port 0 is falsy and means the default 9001, as in the JS server
    let v_port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let v_over = config(
        v_dir.path(),
        json!({
            "dashboard_enable": true, "dashboard_port": v_port, "dashboard_listening_ip": "127.0.0.1",
            "admin_username": "admin", "admin_password": "Secret123!", "session_secret": "a-test-session-secret",
        }),
    );
    // loaded from a file: settings are written next to it
    let mut v_cfg = de_storage_server::config::Config::load(v_dir.path(), "t.config", &|_| None, &|_| {}).unwrap();
    for (k, v) in v_over.raw().as_object().unwrap() {
        v_cfg.set(k, v.clone());
    }
    let v_db = Db::open(&v_dir.path().join("t.db"), Log::silent()).unwrap();
    let v_storage = Storage::new(v_cfg, v_db, Log::silent(), Some(SECRET.into()), None);
    let v_started = de_storage_server::start(&v_storage).await.unwrap();
    let mut c = Client { addr: v_started.dashboard.unwrap(), jar: HashMap::new() };

    let r = c.req("GET", "/", &[], "").await;
    assert_eq!((r.status, r.header("location")), (302, Some("/admin/login")));
    assert_eq!(r.header("x-frame-options"), Some("SAMEORIGIN"), "helmet headers");

    let r = c.req("GET", "/admin/dashboard", &[], "").await;
    assert_eq!((r.status, r.header("location")), (302, Some("/admin/login")));

    let r = c.req("GET", "/admin/login", &[], "").await;
    assert_eq!(r.status, 200);
    assert!(r.body.contains("<title>Storage Admin Login</title>"));
    let v_token = r.csrf();
    let v_form = "application/x-www-form-urlencoded";

    // no token: back to the login page
    let r = c.req("POST", "/admin/login", &[("Content-Type", v_form)], "username=admin&password=Secret123!").await;
    assert_eq!((r.status, r.header("location")), (302, Some("/admin/login")));
    let r = c
        .req(
            "POST",
            "/admin/login",
            &[("Content-Type", v_form)],
            &format!("_csrf={}&username=admin&password=Wrong1234", v_token),
        )
        .await;
    assert_eq!(r.header("location"), Some("/admin/login"));
    assert!(c.req("GET", "/admin/login", &[], "").await.body.contains("Invalid username or password"));
    let r = c
        .req(
            "POST",
            "/admin/login",
            &[("Content-Type", v_form)],
            &format!("_csrf={}&username=admin&password=Secret123!", v_token),
        )
        .await;
    assert_eq!((r.status, r.header("location")), (302, Some("/admin/dashboard")));
    assert!(c.jar.contains_key("connect.sid"));

    let r = c.req("GET", "/admin/dashboard", &[], "").await;
    assert_eq!(r.status, 200);
    assert!(r.body.contains("Welcome, admin"));

    let r = c.req("GET", "/admin/api/stats", &[], "").await;
    assert_eq!(r.json()["error"], 0);
    assert_eq!(r.header("ratelimit-limit"), Some("60"));

    let v_page = c.req("GET", "/admin/settings", &[], "").await;
    let i = v_page.body.find("window.CSRF_TOKEN = \"").unwrap() + 21;
    let v_api_token = v_page.body[i..i + v_page.body[i..].find('"').unwrap()].to_string();
    let r = c
        .req(
            "POST",
            "/admin/api/settings",
            &[("Content-Type", "application/json")],
            r#"{"max_news_per_account":3,"max_missions_per_account":1,"max_missions_per_unit":1}"#,
        )
        .await;
    assert_eq!((r.status, r.json()["errorMessage"].clone()), (403, json!("Invalid or missing CSRF token")));
    let r = c
        .req(
            "POST",
            "/admin/api/settings",
            &[("Content-Type", "application/json"), ("X-CSRF-Token", &v_api_token)],
            r#"{"max_news_per_account":3,"max_missions_per_account":1,"max_missions_per_unit":1}"#,
        )
        .await;
    assert_eq!(r.json()["limits"]["max_news_per_account"], 3);
    let v_local = std::fs::read_to_string(v_dir.path().join("t.config.local")).unwrap();
    assert!(v_local.starts_with("// Auto-generated by storage server admin dashboard."));
    assert!(v_local.contains("\"max_news_per_account\": 3"));
    assert_eq!(v_storage.cfg().get("limits").unwrap()["max_missions_per_unit"], 1, "applied live");

    let r = c.req("POST", "/admin/logout", &[("Content-Type", v_form)], &format!("_csrf={}", v_token)).await;
    assert_eq!(r.header("location"), Some("/admin/login"));
    assert_eq!(c.req("GET", "/admin/dashboard", &[], "").await.status, 302);
}

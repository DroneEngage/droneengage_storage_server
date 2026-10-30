//! Shared test helpers: a storage server on a temporary database with one
//! authenticated comm server connection whose replies are captured.

#![allow(dead_code)]

use de_storage_server::config::Config;
use de_storage_server::db::Db;
use de_storage_server::handlers::Handlers;
use de_storage_server::logger::Log;
use de_storage_server::server::{Conn, Out, Storage};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

pub const SECRET: &str = "storage-test-shared-secret-0123456789";

pub struct Rig {
    pub storage: Arc<Storage>,
    pub handlers: Handlers,
    pub rx: UnboundedReceiver<Out>,
    pub dir: tempfile::TempDir,
}

/// The base configuration merged with p_over.
pub fn config(p_dir: &std::path::Path, p_over: Value) -> Config {
    let mut v = json!({
        "server_id": "DE_StorageSrv_T",
        "server_ip": "127.0.0.1",
        "server_port": 0,
        "enable_SSL": false,
        "s2s_cert_enabled": false,
        "database": { "path": "t.db" },
        "queue": { "maxQueueSize": 1000, "retryInterval": 5000 },
        "dashboard_enable": false,
    });
    if let (Value::Object(b), Value::Object(o)) = (&mut v, p_over) {
        for (k, x) in o {
            if x.is_null() {
                b.remove(&k);
            } else {
                b.insert(k, x);
            }
        }
    }
    Config::from_value(v, p_dir)
}

pub fn rig(p_over: Value) -> Rig {
    let v_dir = tempfile::tempdir().unwrap();
    let v_cfg = config(v_dir.path(), p_over);
    let v_db = Db::open(&v_dir.path().join("t.db"), Log::silent()).unwrap();
    let v_storage = Storage::new(v_cfg, v_db, Log::silent(), Some(SECRET.into()), None);
    let (v_tx, v_rx) = unbounded_channel();
    let mut v_conn = Conn::new(1, "127.0.0.1".into(), v_tx);
    v_conn.authenticated = true;
    v_conn.comm_server_id = Some(json!("comm_test"));
    v_storage.conns().insert(1, v_conn);
    Rig { handlers: Handlers::new(v_storage.clone()), storage: v_storage, rx: v_rx, dir: v_dir }
}

impl Rig {
    /// Everything the connection received so far.
    pub fn sent(&mut self) -> Vec<Value> {
        let mut v_out = Vec::new();
        while let Ok(Out::Text(t)) = self.rx.try_recv() {
            v_out.push(serde_json::from_str(&t).unwrap());
        }
        v_out
    }

    /// Handles one message from the comm server; returns the reply (the
    /// first message sent back; a news push may follow it).
    pub fn send(&mut self, p_msg: Value) -> Value {
        de_storage_server::ws_server::handle_message(&self.storage, &self.handlers, 1, p_msg.to_string().as_bytes());
        self.sent().into_iter().next().unwrap_or(Value::Null)
    }

    /// Changes the live configuration.
    pub fn set(&self, p_key: &str, p_value: Value) {
        self.storage.update_cfg(|c| c.set(p_key, p_value));
    }
}

//! The running storage server (index.js StorageServer + the connection
//! table of websocketServer.js): configuration, database, S2S credentials
//! and the comm server connections.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::Config;
use crate::db::Db;
use crate::logger::{FileLogger, Log};
use crate::s2s_auth::{S2sAuth, S2S_AUTH_METHOD_ED25519, S2S_AUTH_METHOD_HMAC};
use crate::util::{js_stringify, now_ms};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What a connection's writer sends.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Text(String),
    Close(u16, String),
}

/// One comm server connection (connectionInfo).
pub struct Conn {
    pub id: u64,
    pub ip: String,
    pub authenticated: bool,
    /// envelope.id of the accepted response; None is undefined
    pub comm_server_id: Option<Value>,
    pub connected_at: i64,
    /// shared with the auth timeout, which outlives the connection
    pub auth_pending: Arc<AtomicBool>,
    pub nonce: Option<String>,
    m_tx: UnboundedSender<Out>,
    /// ws readyState OPEN
    m_open: bool,
}

impl Conn {
    pub fn new(p_id: u64, p_ip: String, p_tx: UnboundedSender<Out>) -> Conn {
        Conn {
            id: p_id,
            ip: p_ip,
            authenticated: false,
            comm_server_id: Some(Value::Null),
            connected_at: now_ms(),
            auth_pending: Arc::new(AtomicBool::new(true)),
            nonce: None,
            m_tx: p_tx,
            m_open: true,
        }
    }

    pub fn is_open(&self) -> bool {
        self.m_open
    }

    /// ws.send while OPEN.
    pub fn send_text(&self, p_text: String) {
        if self.m_open {
            let _ = self.m_tx.send(Out::Text(p_text));
        }
    }

    /// ws.close(code, reason): a close frame once; no sends after it.
    pub fn close(&mut self, p_code: u16, p_reason: &str) {
        if self.m_open {
            self.m_open = false;
            let _ = self.m_tx.send(Out::Close(p_code, p_reason.to_string()));
        }
    }

    /// The peer started the closing handshake (readyState CLOSING).
    pub fn mark_closing(&mut self) {
        self.m_open = false;
    }
}

pub struct Storage {
    m_cfg: RwLock<Config>,
    pub db: Db,
    pub log: Log,
    pub s2s: S2sAuth,
    pub file_log: Option<FileLogger>,
    m_auth_method: RwLock<&'static str>,
    m_conns: Mutex<BTreeMap<u64, Conn>>,
    m_counter: AtomicU64,
    /// set by stop(): the Node.js server exits before any close event runs
    m_stopping: AtomicBool,
}

impl Storage {
    pub fn new(
        p_cfg: Config,
        p_db: Db,
        p_log: Log,
        p_env_secret: Option<String>,
        p_file_log: Option<FileLogger>,
    ) -> Arc<Storage> {
        let v_method = if p_cfg.is_true("s2s_cert_enabled") { S2S_AUTH_METHOD_ED25519 } else { S2S_AUTH_METHOD_HMAC };
        Arc::new(Storage {
            s2s: S2sAuth::new(&p_cfg, p_env_secret),
            m_cfg: RwLock::new(p_cfg),
            db: p_db,
            log: p_log,
            file_log: p_file_log,
            m_auth_method: RwLock::new(v_method),
            m_conns: Mutex::new(BTreeMap::new()),
            m_counter: AtomicU64::new(0),
            m_stopping: AtomicBool::new(false),
        })
    }

    /// global.m_serverconfig.m_configuration
    pub fn cfg(&self) -> RwLockReadGuard<'_, Config> {
        self.m_cfg.read().unwrap()
    }

    /// Changes the live configuration (dashboard settings, $$HASH$$).
    pub fn update_cfg(&self, p_fn: impl FnOnce(&mut Config)) {
        p_fn(&mut self.m_cfg.write().unwrap());
    }

    pub fn auth_method(&self) -> &'static str {
        *self.m_auth_method.read().unwrap()
    }

    pub fn set_auth_method(&self, p_method: &'static str) {
        *self.m_auth_method.write().unwrap() = p_method;
    }

    /// The port: de_storage_server_port, else server_port.
    pub fn port_text(&self) -> String {
        match std::env::var("de_storage_server_port") {
            Ok(p) if !p.is_empty() => p,
            _ => crate::util::js_key(self.cfg().get("server_port")),
        }
    }

    /// ++connectionCounter
    pub fn next_conn_id(&self) -> u64 {
        self.m_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn conns(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, Conn>> {
        self.m_conns.lock().unwrap()
    }

    pub fn connection_count(&self) -> usize {
        self.conns().len()
    }

    /// send(connectionId, data)
    pub fn send(&self, p_conn: u64, p_data: &Value) {
        if let Some(c) = self.conns().get(&p_conn) {
            c.send_text(js_stringify(p_data));
        }
    }

    /// sendError(connectionId, error)
    pub fn send_error(&self, p_conn: u64, p_error: &str) {
        self.send(p_conn, &json!({ "type": "error", "error": p_error, "timestamp": now_ms() }));
    }

    /// close(connectionId, code, reason)
    pub fn close(&self, p_conn: u64, p_code: u16, p_reason: &str) {
        if let Some(c) = self.conns().get_mut(&p_conn) {
            c.close(p_code, p_reason);
        }
    }

    /// broadcast(data): every authenticated connection, in connection order.
    pub fn broadcast(&self, p_data: &Value) {
        let v_text = js_stringify(p_data);
        for c in self.conns().values() {
            if c.authenticated {
                c.send_text(v_text.clone());
            }
        }
    }

    /// connection.commServerId (None = undefined); None for an unknown connection.
    pub fn comm_server_id(&self, p_conn: u64) -> Option<Value> {
        self.conns().get(&p_conn).and_then(|c| c.comm_server_id.clone())
    }

    pub fn is_stopping(&self) -> bool {
        self.m_stopping.load(Ordering::SeqCst)
    }

    /// stop(): closes every connection with 1001.
    pub fn close_all(&self) {
        self.m_stopping.store(true, Ordering::SeqCst);
        for c in self.conns().values_mut() {
            c.close(1001, "Server shutting down");
        }
    }
}

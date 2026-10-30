//! The WebSocket server comm servers connect to (websocketServer.js).
//!
//! Every connection must pass the S2S handshake first (Ed25519 when
//! s2s_cert_enabled, else the shared-secret HMAC), within 8 s. Messages are
//! JSON; `mt` selects the handler. The server listens over TLS when
//! enable_SSL and both certificate files are configured, else plain.
//!
//! Like Node's http server with only the ws upgrade handler, a plain HTTP
//! request gets no answer (until Node's 300 s request timeout).

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::{sleep_until, Instant};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::handlers::Handlers;
use crate::s2s_auth::{
    build_challenge, generate_nonce, parse_envelope, S2S_AUTH_HANDSHAKE_TIMEOUT_MS, S2S_AUTH_METHOD_ED25519,
    S2S_AUTH_RESPONSE,
};
use crate::server::{Conn, Out, Storage};
use crate::util::{js_interp, js_parse_bytes, js_strict_eq, now_ms};

/// ws closeTimeout: how long a close handshake may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);
/// Node's server.requestTimeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_HEADER_BYTES: usize = 16 * 1024;

pub struct RequestHead {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl RequestHead {
    /// A header value; repeated headers are joined with ", " (Node).
    pub fn header(&self, p_name: &str) -> Option<String> {
        let v: Vec<&str> =
            self.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(p_name)).map(|(_, v)| v.as_str()).collect();
        (!v.is_empty()).then(|| v.join(", "))
    }
}

/// Reads a request head; returns it and any bytes read past it. Err(0) is
/// a closed connection, else the HTTP error code to answer.
pub async fn read_head<S: AsyncRead + Unpin>(p_stream: &mut S) -> Result<(RequestHead, Vec<u8>), u16> {
    let mut v_buf: Vec<u8> = Vec::with_capacity(1024);
    let mut v_chunk = [0u8; 4096];
    loop {
        let n = p_stream.read(&mut v_chunk).await.map_err(|_| 0u16)?;
        if n == 0 {
            return Err(0);
        }
        v_buf.extend_from_slice(&v_chunk[..n]);
        let mut v_headers = [httparse::EMPTY_HEADER; 100];
        let mut v_req = httparse::Request::new(&mut v_headers);
        match v_req.parse(&v_buf) {
            Ok(httparse::Status::Complete(v_len)) => {
                let v_head = RequestHead {
                    method: v_req.method.unwrap_or("").to_string(),
                    url: v_req.path.unwrap_or("/").to_string(),
                    headers: v_req
                        .headers
                        .iter()
                        .map(|h| (h.name.to_string(), String::from_utf8_lossy(h.value).into_owned()))
                        .collect(),
                };
                return Ok((v_head, v_buf[v_len..].to_vec()));
            }
            Ok(httparse::Status::Partial) => {
                if v_buf.len() > MAX_HEADER_BYTES {
                    return Err(431);
                }
            }
            Err(httparse::Error::TooManyHeaders) => return Err(431),
            Err(_) => return Err(400),
        }
    }
}

/// ws's abortHandshake response.
fn http_error(p_code: u16, p_extra: &str) -> Vec<u8> {
    let v_reason = match p_code {
        400 => "Bad Request",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        _ => "Bad Request",
    };
    format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\nContent-Type: text/html\r\nContent-Length: {}\r\n{}\r\n{}",
        p_code,
        v_reason,
        v_reason.len(),
        p_extra,
        v_reason
    )
    .into_bytes()
}

/// Validates an upgrade like ws's handleUpgrade; the 101 response or the
/// error response.
pub fn upgrade_response(p_head: &RequestHead) -> Result<Vec<u8>, Vec<u8>> {
    if p_head.method != "GET" {
        return Err(http_error(405, ""));
    }
    if !p_head.header("upgrade").map(|u| u.trim().eq_ignore_ascii_case("websocket")).unwrap_or(false) {
        return Err(http_error(400, ""));
    }
    let v_key = p_head.header("sec-websocket-key").unwrap_or_default();
    let v_key_ok = v_key.len() == 24
        && v_key.ends_with("==")
        && v_key[..22].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/');
    if !v_key_ok {
        return Err(http_error(400, ""));
    }
    let v_version = p_head.header("sec-websocket-version").unwrap_or_default();
    if v_version.trim() != "13" && v_version.trim() != "8" {
        return Err(http_error(400, "Sec-WebSocket-Version: 13, 8\r\n"));
    }
    let mut v_resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n",
        derive_accept_key(v_key.as_bytes())
    );
    if let Some(v_protocols) = p_head.header("sec-websocket-protocol") {
        if let Some(v_first) = v_protocols.split(',').map(str::trim).find(|p| !p.is_empty()) {
            v_resp.push_str(&format!("Sec-WebSocket-Protocol: {}\r\n", v_first));
        }
    }
    v_resp.push_str("\r\n");
    Ok(v_resp.into_bytes())
}

/// req.socket.remoteAddress as Node prints it.
pub fn remote_address(p_addr: &SocketAddr) -> String {
    p_addr.ip().to_string()
}

/// listen(port, host) like Node: a missing host is all interfaces.
pub async fn bind_listener(p_ip: &str, p_port: &str) -> Result<TcpListener, String> {
    let v_port: u16 = if p_port == "undefined" || p_port.is_empty() {
        0
    } else {
        p_port.trim().parse().map_err(|_| format!("options.port should be >= 0 and < 65536. Received {}.", p_port))?
    };
    let v_host = if p_ip == "undefined" || p_ip.is_empty() { "::" } else { p_ip };
    let v_host = v_host.trim_start_matches('[').trim_end_matches(']');
    let v_addrs: Vec<SocketAddr> =
        tokio::net::lookup_host((v_host, v_port)).await.map_err(|e| e.to_string())?.collect();
    let mut v_last = String::from("no address");
    for v_addr in v_addrs {
        match TcpListener::bind(v_addr).await {
            Ok(l) => return Ok(l),
            Err(e) => {
                v_last = if e.kind() == std::io::ErrorKind::AddrInUse {
                    format!("listen EADDRINUSE: address already in use {}", v_addr)
                } else {
                    format!("listen {}: {}", v_addr, e)
                }
            }
        }
    }
    Err(v_last)
}

/// sendAuthChallenge: the nonce challenge and its 8 s timeout. The timeout
/// outlives the connection, as the JavaScript timer does.
fn send_auth_challenge(p_storage: &Arc<Storage>, p_conn: u64) {
    let v_nonce = generate_nonce();
    let v_pending = {
        let mut v_conns = p_storage.conns();
        let Some(c) = v_conns.get_mut(&p_conn) else { return };
        c.nonce = Some(v_nonce.clone());
        c.send_text(build_challenge(&v_nonce, Some(p_storage.auth_method())));
        c.auth_pending.clone()
    };
    p_storage.log.debug(&format!("Sent S2S auth challenge to connection {}", p_conn));
    let v_storage = p_storage.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(S2S_AUTH_HANDSHAKE_TIMEOUT_MS)).await;
        if v_pending.load(Ordering::SeqCst) {
            v_storage.log.warn(&format!("S2S auth timeout for connection {}", p_conn));
            v_storage.close(p_conn, 1008, "Auth timeout");
        }
    });
}

/// handleS2SAuthResponse
fn handle_auth_response(p_storage: &Arc<Storage>, p_conn: u64, p_env: &Value) {
    let Some(v_nonce) = p_storage.conns().get(&p_conn).map(|c| c.nonce.clone()) else { return };
    let v_nonce = v_nonce.unwrap_or_default();
    let v_valid = if p_storage.auth_method() == S2S_AUTH_METHOD_ED25519 {
        p_storage.s2s.verify(&v_nonce, p_env.get("sig"), p_env.get("id"))
    } else {
        p_storage.s2s.verify_hmac(&v_nonce, p_env.get("sig"))
    };
    if v_valid {
        let v_id = p_env.get("id").cloned();
        if let Some(c) = p_storage.conns().get_mut(&p_conn) {
            c.authenticated = true;
            c.comm_server_id = v_id.clone();
            c.auth_pending.store(false, Ordering::SeqCst);
        }
        p_storage.log.info(&format!(
            "Connection {} authenticated for comm server {}",
            p_conn,
            js_interp(v_id.as_ref())
        ));
        p_storage.send(p_conn, &json!({ "type": "auth_success", "connectionId": p_conn, "timestamp": now_ms() }));
    } else {
        p_storage.log.warn(&format!("S2S auth failed for connection {}", p_conn));
        p_storage.send_error(p_conn, "Invalid signature");
        p_storage.close(p_conn, 1008, "Authentication failed");
    }
}

/// handleMessage
pub fn handle_message(p_storage: &Arc<Storage>, p_handlers: &Handlers, p_conn: u64, p_data: &[u8]) {
    let Some(v_authenticated) = p_storage.conns().get(&p_conn).map(|c| c.authenticated) else {
        p_storage.log.warn(&format!("Message from unknown connection {}", p_conn));
        return;
    };
    if let Some(v_env) = parse_envelope(p_data) {
        if js_strict_eq(v_env.get("s2s_auth"), Some(&json!(S2S_AUTH_RESPONSE))) {
            handle_auth_response(p_storage, p_conn, &v_env);
        }
        return;
    }
    let v_message = match js_parse_bytes(p_data) {
        Some(Value::Null) => Err("Cannot read properties of null (reading 'mt')".to_string()),
        Some(m) => Ok(m),
        None => Err(json_error(p_data)),
    };
    let v_message = match v_message {
        Ok(m) => m,
        Err(e) => {
            p_storage.log.error(&format!("Error handling message from {}: {}", p_conn, e));
            p_storage.send_error(p_conn, "Invalid message format");
            return;
        }
    };
    let v_mt = crate::util::field(&v_message, "mt");
    p_storage.log.debug(&format!("Message from connection {}, type: {}", p_conn, js_interp(v_mt)));
    if !v_authenticated {
        p_storage.send_error(p_conn, "Not authenticated");
        p_storage.close(p_conn, 1008, "Authentication required");
        return;
    }
    match Handlers::lookup(v_mt) {
        Some(t) => p_handlers.handle(t, p_conn, &v_message),
        None => {
            p_storage.log.warn(&format!("Unknown message type: {}", js_interp(v_mt)));
            p_storage.send_error(p_conn, &format!("Unknown message type: {}", js_interp(v_mt)));
        }
    }
}

/// The text of a JSON.parse failure (logged only).
fn json_error(p_data: &[u8]) -> String {
    let v_text = String::from_utf8_lossy(p_data);
    if v_text.trim().is_empty() {
        return "Unexpected end of JSON input".into();
    }
    match serde_json::from_str::<Value>(&v_text) {
        Err(e) => format!("{} is not valid JSON ({})", serde_json::to_string(&*v_text).unwrap_or_default(), e),
        Ok(_) => "Unexpected token".into(),
    }
}

/// One connection: HTTP head, upgrade, then the comm server's messages.
async fn serve<S>(p_storage: Arc<Storage>, p_handlers: Arc<Handlers>, mut p_stream: S, p_remote: SocketAddr)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (v_head, v_rest) = match read_head(&mut p_stream).await {
        Ok(h) => h,
        Err(0) => return,
        Err(code) => {
            let _ = p_stream.write_all(&http_error(code, "")).await;
            return;
        }
    };
    if v_head.header("upgrade").is_none() {
        // no request handler: Node answers only when the request times out
        let mut v_sink = [0u8; 1024];
        let _ = tokio::time::timeout(REQUEST_TIMEOUT, async {
            while let Ok(n) = p_stream.read(&mut v_sink).await {
                if n == 0 {
                    break;
                }
            }
        })
        .await;
        let _ = p_stream.write_all(b"HTTP/1.1 408 Request Timeout\r\nConnection: close\r\n\r\n").await;
        return;
    }
    match upgrade_response(&v_head) {
        Ok(v_resp) => {
            if p_stream.write_all(&v_resp).await.is_err() {
                return;
            }
        }
        Err(v_resp) => {
            let _ = p_stream.write_all(&v_resp).await;
            let _ = p_stream.shutdown().await;
            return;
        }
    }
    let v_ws =
        WebSocketStream::from_partially_read(p_stream, v_rest, Role::Server, Some(crate::net::ws_config())).await;
    run_connection(p_storage, p_handlers, v_ws, p_remote).await;
}

/// handleConnection and the connection's lifetime.
async fn run_connection<S>(
    p_storage: Arc<Storage>,
    p_handlers: Arc<Handlers>,
    p_ws: WebSocketStream<S>,
    p_remote: SocketAddr,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let v_id = p_storage.next_conn_id();
    let v_ip = remote_address(&p_remote);
    p_storage.log.info(&format!("New connection from {}, connectionId: {}", v_ip, v_id));
    let (v_tx, mut v_rx) = unbounded_channel::<Out>();
    let v_connected_at = now_ms();
    p_storage.conns().insert(v_id, Conn::new(v_id, v_ip, v_tx));

    let (mut v_sink, mut v_source) = p_ws.split();
    let (v_closing_tx, mut v_closing_rx) = tokio::sync::watch::channel(false);
    let v_writer = tokio::spawn(async move {
        while let Some(v_out) = v_rx.recv().await {
            let v_msg = match v_out {
                Out::Text(t) => Message::Text(t.into()),
                Out::Close(code, reason) => {
                    let _ = v_closing_tx.send(true);
                    Message::Close(Some(CloseFrame { code: CloseCode::from(code), reason: reason.into() }))
                }
            };
            let v_close = matches!(v_msg, Message::Close(_));
            if v_sink.send(v_msg).await.is_err() || v_close {
                break;
            }
        }
        v_sink
    });

    send_auth_challenge(&p_storage, v_id);

    let mut v_code: u16 = 1006;
    let mut v_reason = String::new();
    let mut v_close_deadline: Option<Instant> = None;
    let v_far = Instant::now() + Duration::from_secs(86400 * 365);
    loop {
        tokio::select! {
            v_in = v_source.next() => {
                let Some(Ok(v_msg)) = v_in else { break };
                match &v_msg {
                    Message::Text(t) => handle_message(&p_storage, &p_handlers, v_id, t.as_bytes()),
                    Message::Binary(b) => handle_message(&p_storage, &p_handlers, v_id, b),
                    Message::Close(v_frame) => {
                        if let Some(c) = p_storage.conns().get_mut(&v_id) {
                            c.mark_closing();
                        }
                        match v_frame {
                            Some(f) => {
                                v_code = u16::from(f.code);
                                v_reason = f.reason.to_string();
                            }
                            None => v_code = 1005,
                        }
                    }
                    _ => {}
                }
            },
            v_changed = v_closing_rx.changed(), if v_close_deadline.is_none() => {
                if v_changed.is_ok() && *v_closing_rx.borrow() {
                    v_close_deadline = Some(Instant::now() + CLOSE_TIMEOUT);
                }
            },
            _ = sleep_until(v_close_deadline.unwrap_or(v_far)), if v_close_deadline.is_some() => break,
        }
    }
    v_writer.abort();
    // handleDisconnection
    if p_storage.conns().remove(&v_id).is_some() && !p_storage.is_stopping() {
        p_storage.log.info(&format!(
            "Connection {} disconnected (code: {}, reason: {}, duration: {}ms)",
            v_id,
            v_code,
            v_reason,
            now_ms() - v_connected_at
        ));
    }
}

/// start(): validates the S2S mode, then listens.
pub async fn start(p_storage: &Arc<Storage>) -> Result<WsListener, String> {
    let v_cfg = p_storage.cfg().clone();
    if v_cfg.is_true("s2s_cert_enabled") {
        let v_empty = match v_cfg.get("s2s_trusted_server_keys") {
            None | Some(Value::Null) => true,
            Some(Value::Object(m)) => m.is_empty(),
            Some(Value::Array(a)) => a.is_empty(),
            Some(Value::String(s)) => s.is_empty(),
            Some(_) => true,
        };
        if v_empty {
            p_storage.log.error("s2s_cert_enabled=true but s2s_trusted_server_keys is empty — EVERY connection will be rejected (silent lockout). Configure trusted public keys or set s2s_cert_enabled=false for shared-secret mode.");
        }
        p_storage.set_auth_method(S2S_AUTH_METHOD_ED25519);
    } else {
        if let Err(e) = p_storage.s2s.shared_secret_with(true) {
            p_storage.log.error(&format!("S2S shared-secret configuration error: {}", e));
            std::process::exit(1);
        }
        p_storage.set_auth_method(crate::s2s_auth::S2S_AUTH_METHOD_HMAC);
        p_storage.log.info("S2S shared-secret (HMAC) authentication enabled");
    }

    let mut v_acceptor = None;
    if crate::util::js_truthy(v_cfg.get("enable_SSL")) {
        let v_cert = v_cfg.get("ssl_cert_file").filter(|v| crate::util::js_truthy(Some(v)));
        let v_key = v_cfg.get("ssl_key_file").filter(|v| crate::util::js_truthy(Some(v)));
        if let (Some(v_cert), Some(v_key)) = (v_cert, v_key) {
            let v_cert = v_cfg.resolve(&crate::util::js_to_string(v_cert));
            let v_key = v_cfg.resolve(&crate::util::js_to_string(v_key));
            if !v_cert.exists() {
                p_storage.log.error(&format!("SSL certificate file not found: {}", v_cert.display()));
                std::process::exit(1);
            }
            if !v_key.exists() {
                p_storage.log.error(&format!("SSL key file not found: {}", v_key.display()));
                std::process::exit(1);
            }
            v_acceptor = Some(crate::net::tls_acceptor(&v_key, &v_cert)?);
            p_storage.log.info("SSL/TLS enabled for WebSocket server");
        } else {
            p_storage.log.warn("SSL enabled but cert/key files not configured, falling back to HTTP");
        }
    }

    let v_host = js_interp(v_cfg.get("server_ip"));
    let v_port = p_storage.port_text();
    let v_listener = bind_listener(&v_host, &v_port).await;
    Ok(WsListener {
        m_storage: p_storage.clone(),
        m_handlers: Arc::new(Handlers::new(p_storage.clone())),
        m_listener: v_listener,
        m_acceptor: v_acceptor,
        m_host: v_host,
        m_port: v_port,
    })
}

/// The bound (or failed) listener; `run` reports it — Node's 'listening'
/// or 'error' event, which fires after the rest of startup — and accepts.
pub struct WsListener {
    m_storage: Arc<Storage>,
    m_handlers: Arc<Handlers>,
    m_listener: Result<TcpListener, String>,
    m_acceptor: Option<tokio_rustls::TlsAcceptor>,
    m_host: String,
    m_port: String,
}

impl WsListener {
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.m_listener.as_ref().ok().and_then(|l| l.local_addr().ok())
    }

    pub fn run(self) {
        let v_listener = match self.m_listener {
            Ok(l) => l,
            Err(e) => {
                self.m_storage.log.error(&format!("WebSocket server error: {}", e));
                return;
            }
        };
        self.m_storage.log.info(&format!("WebSocket server listening on {}:{}", self.m_host, self.m_port));
        let (v_storage, v_handlers, v_acceptor) = (self.m_storage, self.m_handlers, self.m_acceptor);
        tokio::spawn(async move {
            loop {
                let Ok((v_tcp, v_remote)) = v_listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                };
                let _ = v_tcp.set_nodelay(true);
                let (v_storage, v_handlers, v_acceptor) = (v_storage.clone(), v_handlers.clone(), v_acceptor.clone());
                tokio::spawn(async move {
                    match v_acceptor {
                        Some(a) => {
                            if let Ok(v_tls) = a.accept(v_tcp).await {
                                serve(v_storage, v_handlers, v_tls, v_remote).await;
                            }
                        }
                        None => serve(v_storage, v_handlers, v_tcp, v_remote).await,
                    }
                });
            }
        });
    }
}

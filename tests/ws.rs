//! The WebSocket server end to end: handshake, routing, broadcasts.

mod common;

use common::*;
use de_storage_server::db::Db;
use de_storage_server::logger::Log;
use de_storage_server::s2s_auth::hmac_with;
use de_storage_server::Storage;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next(p_ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(3), p_ws.next())
            .await
            .expect("a message")
            .expect("open")
            .unwrap()
        {
            Message::Text(t) => return serde_json::from_str(t.as_str()).unwrap(),
            Message::Close(f) => return json!({ "close": f.map(|f| u16::from(f.code)) }),
            _ => {}
        }
    }
}

async fn start() -> (std::net::SocketAddr, tempfile::TempDir) {
    let v_dir = tempfile::tempdir().unwrap();
    let v_db = Db::open(&v_dir.path().join("t.db"), Log::silent()).unwrap();
    let v_storage = Storage::new(config(v_dir.path(), json!({})), v_db, Log::silent(), Some(SECRET.into()), None);
    let v_started = de_storage_server::start(&v_storage).await.unwrap();
    (v_started.ws.unwrap(), v_dir)
}

async fn client(p_addr: std::net::SocketAddr, p_good: bool, p_id: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{}", p_addr)).await.unwrap();
    let v_challenge = next(&mut ws).await;
    assert_eq!(v_challenge["s2s_auth"], "challenge");
    assert_eq!(v_challenge["method"], "hmac-sha256");
    let v_nonce = v_challenge["nonce"].as_str().unwrap();
    let v_sig = if p_good { hmac_with(SECRET, v_nonce) } else { "bad".into() };
    let v_resp = json!({"s2s_auth": "response", "sig": v_sig, "id": p_id}).to_string();
    ws.send(Message::Text(v_resp.into())).await.unwrap();
    ws
}

#[tokio::test]
async fn handshake_routing_and_broadcast() {
    let (v_addr, _dir) = start().await;
    let mut a = client(v_addr, true, "CS1").await;
    assert_eq!(next(&mut a).await["type"], "auth_success");
    let mut b = client(v_addr, true, "CS2").await;
    assert_eq!(next(&mut b).await["type"], "auth_success");

    a.send(Message::Text(
        json!({"mt": 9002, "rid": 5, "ms": {"unitId": "U1", "tasks": [{"taskId": "t1", "data": {"a": 1}}]}})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let v = next(&mut a).await;
    assert_eq!(
        (v["mt"].clone(), v["rid"].clone(), v["ms"]["savedTasks"].clone()),
        (json!(9002), json!(5), json!(["t1"]))
    );
    a.send(Message::Text(json!({"mt": 9001, "ms": {"unitId": "U1"}}).to_string().into())).await.unwrap();
    assert_eq!(next(&mut a).await["ms"]["tasks"][0]["data"], json!({"a": 1}));

    // a news change reaches every authenticated comm server
    a.send(Message::Text(
        json!({"mt": 9016, "ms": {"newsId": "n1", "scope": "global", "body": "hi"}}).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(next(&mut a).await["mt"], 9016);
    assert_eq!(next(&mut a).await["mt"], 9018);
    let v = next(&mut b).await;
    assert_eq!((v["mt"].clone(), v["ms"]["news"]["body"].clone()), (json!(9018), json!("hi")));

    a.send(Message::Text("{bad".into())).await.unwrap();
    assert_eq!(next(&mut a).await["error"], "Invalid message format");
    a.send(Message::Text(json!({"mt": 1}).to_string().into())).await.unwrap();
    assert_eq!(next(&mut a).await["error"], "Unknown message type: 1");
}

#[tokio::test]
async fn unauthenticated_and_bad_signatures_are_closed() {
    let (v_addr, _dir) = start().await;
    let mut v_bad = client(v_addr, false, "X").await;
    assert_eq!(next(&mut v_bad).await["error"], "Invalid signature");
    assert_eq!(next(&mut v_bad).await["close"], 1008);

    let (mut v_anon, _) = tokio_tungstenite::connect_async(format!("ws://{}", v_addr)).await.unwrap();
    assert_eq!(next(&mut v_anon).await["s2s_auth"], "challenge");
    v_anon.send(Message::Text(json!({"mt": 9001}).to_string().into())).await.unwrap();
    assert_eq!(next(&mut v_anon).await["error"], "Not authenticated");
    assert_eq!(next(&mut v_anon).await["close"], 1008);
}

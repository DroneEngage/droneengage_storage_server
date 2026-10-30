//! The shared-secret (HMAC) mode (test_s2s_auth.js).

use de_storage_server::config::Config;
use de_storage_server::s2s_auth::{build_challenge, hmac_with, parse_envelope, S2sAuth};
use serde_json::{json, Value};

const SECRET: &str = "a-perfectly-fine-secret-123";

#[test]
fn roundtrip_and_rejections() {
    let v_auth = S2sAuth::new(&Config::from_value(json!({}), "."), Some(SECRET.into()));
    let v_sig = hmac_with(SECRET, "nonce-1");
    assert!(v_auth.verify_hmac("nonce-1", Some(&json!(v_sig))));
    assert!(!v_auth.verify_hmac("nonce-1", Some(&json!(hmac_with("other-secret-0123456", "nonce-1")))));
    assert!(!v_auth.verify_hmac("nonce-1", Some(&json!(format!("{}x", v_sig)))));
    assert!(!v_auth.verify_hmac("nonce-1", None));
}

#[test]
fn bad_secrets_fail_closed() {
    let a = |cfg: Value, env: Option<&str>| {
        S2sAuth::new(&Config::from_value(cfg, "."), env.map(str::to_string)).shared_secret()
    };
    assert!(a(json!({}), None).unwrap_err().contains("no secret is configured"));
    assert!(a(json!({}), Some("changeme")).unwrap_err().contains("placeholder"));
    assert!(a(json!({}), Some("tooshort")).unwrap_err().contains("too short"));
    let v_dir = tempfile::tempdir().unwrap();
    let v_cfg = json!({"s2s_shared_secret_file": "nope.key"});
    let e = S2sAuth::new(&Config::from_value(v_cfg, v_dir.path()), None).shared_secret().unwrap_err();
    assert!(e.starts_with("S2S shared secret file not found"));
}

#[test]
fn auto_generates_a_private_secret_file() {
    let v_dir = tempfile::tempdir().unwrap();
    let v_cfg = Config::from_value(json!({"s2s_shared_secret_file": "s.key"}), v_dir.path());
    let v_secret = S2sAuth::new(&v_cfg, None).shared_secret_with(true).unwrap();
    assert_eq!(v_secret.len(), 64);
    let v_file = v_dir.path().join("s.key");
    assert_eq!(std::fs::read_to_string(&v_file).unwrap(), format!("{}\n", v_secret));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&v_file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // the next start reads it back
    assert_eq!(S2sAuth::new(&v_cfg, None).shared_secret().unwrap(), v_secret);
}

#[test]
fn envelopes() {
    let v: Value = serde_json::from_str(&build_challenge("n", Some("hmac-sha256"))).unwrap();
    assert_eq!(v, json!({"s2s_auth": "challenge", "nonce": "n", "method": "hmac-sha256"}));
    let v_resp = json!({"s2s_auth": "response", "sig": hmac_with(SECRET, "n"), "id": "C"}).to_string();
    assert_eq!(parse_envelope(v_resp.as_bytes()).unwrap()["id"], "C");
    assert!(parse_envelope(b" {\"s2s_auth\":1}").is_none(), "must start with '{{'");
}

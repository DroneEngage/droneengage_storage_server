//! The storage server's configuration: the shipped file and its
//! environment overrides.

use de_storage_server::config::{load, DEFAULT_CONFIG_FILENAME};
use serde_json::json;
use std::path::Path;

#[test]
fn shipped_config_parses() {
    let v_cfg = load(Path::new(env!("CARGO_MANIFEST_DIR")), DEFAULT_CONFIG_FILENAME, &|_| None, &|_| {}).unwrap();
    assert_eq!(v_cfg.get("server_id"), Some(&json!("DE_StorageSrv")));
    assert_eq!(v_cfg.get("server_port"), Some(&json!(9000)));
    assert_eq!(v_cfg.get("database").unwrap()["path"], json!("./data/storage.db"));
    assert_eq!(v_cfg.get("limits").unwrap()["max_mission_bytes"], json!(1048576));
}

#[test]
fn env_overrides_like_js_server_config() {
    let v_dir = tempfile::tempdir().unwrap();
    std::fs::write(v_dir.path().join("a.config"), "{ \"k\": 1 }").unwrap();
    let v_env = |k: &str| match k {
        "de_storage_debug_logging" => Some("1".to_string()),
        "de_storage_max_news_per_account" => Some("7x".to_string()),
        "de_storage_max_missions_per_unit" => Some("abc".to_string()),
        "de_storage_max_mission_bytes" => Some(String::new()),
        _ => None,
    };
    let v_cfg = load(v_dir.path(), "a.config", &v_env, &|_| {}).unwrap();
    assert_eq!(
        v_cfg.raw(),
        &json!({"k": 1, "debug_logging": true, "limits": {"max_news_per_account": 7, "max_missions_per_unit": null}})
    );
}

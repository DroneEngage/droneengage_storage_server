//! Server configuration: droneengage_server_common's loader with this
//! server's environment overrides (js_serverConfig.js): de_storage_debug_logging
//! sets `debug_logging`; the quota knobs set `limits.*` (`parseInt`; an empty
//! value is ignored, a non-number is NaN, which JSON can only hold as null
//! and which acts the same: no quota).

pub use droneengage_server_common::config::*;

use serde_json::{Map, Value};
use std::path::Path;

fn debug_logging(p_cfg: &mut Map<String, Value>, p_value: &str) {
    p_cfg.insert("debug_logging".into(), Value::Bool(p_value == "true" || p_value == "1"));
}

/// `if (val) { cfg.limits = cfg.limits || {}; cfg.limits[key] = parseInt(val); }`
fn limit(p_cfg: &mut Map<String, Value>, p_key: &str, p_value: &str) {
    if p_value.is_empty() {
        return;
    }
    if !crate::util::js_truthy(p_cfg.get("limits")) {
        p_cfg.insert("limits".into(), Value::Object(Map::new()));
    }
    let v_num = crate::util::js_parse_int(Some(&Value::String(p_value.into()))).map(Value::from).unwrap_or(Value::Null);
    if let Some(Value::Object(l)) = p_cfg.get_mut("limits") {
        l.insert(p_key.into(), v_num);
    }
}

/// js_serverConfig.js's envOverrides.
pub const ENV_OVERRIDES: [(&str, EnvOverride); 5] = [
    ("de_storage_debug_logging", EnvOverride::Transform(debug_logging)),
    ("de_storage_max_news_per_account", EnvOverride::Transform(|c, v| limit(c, "max_news_per_account", v))),
    ("de_storage_max_missions_per_account", EnvOverride::Transform(|c, v| limit(c, "max_missions_per_account", v))),
    ("de_storage_max_missions_per_unit", EnvOverride::Transform(|c, v| limit(c, "max_missions_per_unit", v))),
    ("de_storage_max_mission_bytes", EnvOverride::Transform(|c, v| limit(c, "max_mission_bytes", v))),
];

/// Loads the storage server's configuration (hash handling is the
/// dashboard's, at its start).
pub fn load(
    p_base_dir: &Path,
    p_file_name: &str,
    p_env: &dyn Fn(&str) -> Option<String>,
    p_log: &dyn Fn(&str),
) -> Result<Config, ConfigError> {
    Config::load_with(
        p_base_dir,
        p_file_name,
        &LoadOptions { env: p_env, log: p_log, overrides: &ENV_OVERRIDES, hash_handling: false },
    )
}

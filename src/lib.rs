//! DroneEngage storage server: persists tasks, missions and news for the
//! comm servers, which reach it over an S2S-authenticated WebSocket, and
//! serves an admin dashboard.
//!
//! A Rust port of the Node.js server: the same server.config, database file,
//! wire protocol and console output.

pub mod cli;
pub mod config;
pub mod dashboard;
pub mod db;
pub mod handlers;
pub mod logger;
pub mod offline_queue;
pub mod server;
pub mod ws_server;

// shared with the other DroneEngage servers
pub use droneengage_server_common::{colors, js, net, s2s_auth, util};

use std::sync::Arc;

pub use config::Config;
pub use server::Storage;

/// What `start` leaves running.
pub struct Started {
    pub ws: Option<std::net::SocketAddr>,
    pub dashboard: Option<std::net::SocketAddr>,
}

/// `DB size: x.xx` (Number.prototype.toFixed(2)).
fn to_fixed2(p_value: f64) -> String {
    format!("{:.2}", p_value)
}

/// printStats
pub fn print_stats(p_storage: &Storage) {
    let v_res = (|| -> Result<(), String> {
        let v_db = p_storage.db.get_stats()?;
        let v_queue = p_storage.db.get_queue_stats()?;
        let g = |v: &serde_json::Value, k: &str| util::js_interp(v.get(k));
        let v_size = v_db.get("dbSize").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let log = &p_storage.log;
        log.info("=== Server Statistics ===");
        log.info(&format!("Database size: {} MB", to_fixed2(v_size / 1024.0 / 1024.0)));
        log.info(&format!("Units: {}", g(&v_db, "units")));
        log.info(&format!("Tasks: {} ({} disabled)", g(&v_db, "tasks"), g(&v_db, "disabledTasks")));
        log.info(&format!(
            "Queued messages: {} for {} units",
            g(&v_queue, "pendingMessages"),
            g(&v_queue, "affectedUnits")
        ));
        log.info(&format!("Active connections: {}", p_storage.connection_count()));
        log.info("========================");
        Ok(())
    })();
    if let Err(e) = v_res {
        p_storage.log.error(&format!("Error getting stats: {}", e));
    }
}

/// StorageServer.start() after the database is open: offline queue, the
/// WebSocket server, statistics, the dashboard. Returns the listening
/// addresses (tests use port 0).
pub async fn start(p_storage: &Arc<Storage>) -> Result<Started, String> {
    // OfflineQueue's constructor reads config.queue.maxQueueSize
    if util::js_nullish(p_storage.cfg().get("queue")) {
        return Err(format!(
            "Cannot read properties of {} (reading 'maxQueueSize')",
            if p_storage.cfg().get("queue").is_some() { "null" } else { "undefined" }
        ));
    }
    offline_queue::start_cleanup_job(p_storage.clone());
    p_storage.log.info("Initializing WebSocket server...");
    let v_ws = ws_server::start(p_storage).await?;
    let log = &p_storage.log;
    log.info("Storage server started successfully");
    let v_host = util::js_interp(p_storage.cfg().get("public_host"));
    log.info(&format!("Server endpoint: {}:{}", v_host, p_storage.port_text()));
    let v_db_path = p_storage.cfg().get("database").and_then(|d| d.get("path")).cloned();
    log.info(&format!("Database: {}", util::js_interp(v_db_path.as_ref())));
    print_stats(p_storage);

    let v_dashboard = if util::js_truthy(p_storage.cfg().get("dashboard_enable")) {
        dashboard::start(p_storage).await?
    } else {
        None
    };
    let v_ws_addr = v_ws.local_addr();
    v_ws.run();
    let v_dashboard_addr = v_dashboard.map(|d| d.run());
    Ok(Started { ws: v_ws_addr, dashboard: v_dashboard_addr.flatten() })
}

/// stop(): closes the connections and the database.
pub async fn stop(p_storage: &Storage) {
    p_storage.log.info("Stopping storage server...");
    p_storage.close_all();
    // let the close frames go out
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    p_storage.db.close();
    p_storage.log.info("Storage server stopped");
}

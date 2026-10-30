//! The server binary (src/server.js): arguments, configuration, banner,
//! memory watchdog, then StorageServer.start(); SIGTERM shuts down
//! gracefully, SIGINT exits at once (server.js's own handler runs first).

use std::path::PathBuf;
use std::time::Duration;

use crate::colors as c;
use crate::config::{Args, Config, DEFAULT_CONFIG_FILENAME};
use crate::db::Db;
use crate::logger::{FileLogger, Log};
use crate::server::{Storage, VERSION};
use crate::util::{js_interp, now_ms};

fn display_help() {
    println!("==================================");
    println!("{}DroneEngage Storage Server version \"{}\"{}", c::BRIGHT, VERSION, c::RESET);
    println!("----------------------------------");
    println!("--config=config_filename config file ");
    println!("-h help ");
    println!("-v version");
    println!("==================================");
}

/// fn_displayInfo: the banner; creates the file logger unless ignoreLog.
fn display_info(p_cfg: &Config) -> Option<FileLogger> {
    let v_port = match std::env::var("de_storage_server_port") {
        Ok(p) if !p.is_empty() => p,
        _ => js_interp(p_cfg.get("server_port")),
    };
    println!("=============================================");
    println!("{}DE Storage Server version \"{}\"{}", c::BRIGHT, VERSION, c::RESET);
    println!("---------------------------------------------");
    println!("Server Name  {}{}{}", c::B_SUCCESS, js_interp(p_cfg.get("server_id")), c::RESET);
    println!(
        "listening on ip: {}{}{} port: {}{}{}",
        c::B_SUCCESS,
        js_interp(p_cfg.get("server_ip")),
        c::RESET,
        c::B_SUCCESS,
        v_port,
        c::RESET
    );
    println!(
        "Auth Server ip: {}{}{} port: {}{}{}",
        c::B_SUCCESS,
        js_interp(p_cfg.get("s2s_ws_target_ip")),
        c::RESET,
        c::B_SUCCESS,
        js_interp(p_cfg.get("s2s_ws_target_port")),
        c::RESET
    );
    let v_logger = if !p_cfg.is_false("ignoreLog") {
        println!("logging is {}disabled{}", c::B_FG_YELLOW, c::RESET);
        None
    } else {
        let v_dir = match p_cfg.get("log_directory").filter(|v| !v.is_null()) {
            Some(d) => p_cfg.resolve(&crate::util::js_to_string(d)),
            None => p_cfg.resolve("./log"),
        };
        let v_tz = p_cfg.get("log_timeZone").filter(|v| !v.is_null()).map(crate::util::js_to_string);
        let v_logger = FileLogger::new(v_dir, p_cfg.is_true("log_detailed"), Some(v_tz.as_deref().unwrap_or("GMT")));
        println!("logging is {}enabled{}", c::B_FG_YELLOW, c::RESET);
        v_logger.info("System Started.", None, None);
        Some(v_logger)
    };
    // console.log("%s", new Date()) prints the ISO form
    println!("Datetime: {}", crate::util::iso_time(now_ms()));
    println!("=====================================================================");
    v_logger
}

/// Resident set size of this process in MB (process.memoryUsage().rss).
fn rss_mb() -> Option<f64> {
    let v_status = std::fs::read_to_string("/proc/self/status").ok()?;
    let v_line = v_status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let v_kb: f64 = v_line.split_whitespace().nth(1)?.parse().ok()?;
    Some((v_kb / 1024.0 * 100.0).round() / 100.0)
}

/// checkMemory: every minute; above memory_max (MB) the process exits with
/// code 1 so its supervisor restarts it.
fn check_memory(p_storage: &Storage) {
    let Some(v_rss) = rss_mb() else { return };
    if p_storage.log.debug_enabled() {
        println!("rss {} MB - ", v_rss);
    }
    if let Some(v_max) = p_storage.cfg().get("memory_max").filter(|v| !v.is_null()) {
        if crate::util::js_number(Some(v_max)) < v_rss {
            println!("Memory is {}{}{} RESTART{}", c::FG_YELLOW, v_rss, c::ERROR, c::RESET);
            std::process::exit(1);
        }
    }
}

/// Runs the server until SIGINT or SIGTERM.
pub async fn run() {
    let v_argv: Vec<String> = std::env::args().skip(1).collect();
    let v_args = Args::parse(&v_argv);
    if v_args.has("h") || v_args.has("help") {
        display_help();
        std::process::exit(0);
    }
    if v_args.has("v") || v_args.has("version") {
        println!("DroneEngage Storage Server version: \"{}\"", VERSION);
        std::process::exit(0);
    }
    let v_file = v_args.get("config").map(str::to_string).unwrap_or_else(|| DEFAULT_CONFIG_FILENAME.to_string());

    let v_root: PathBuf = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let v_cfg = match Config::load(&v_root, &v_file, &|k| std::env::var(k).ok(), &|l| println!("{}", l)) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    let v_log = Log::new(v_cfg.is_true("debug_logging"));
    let v_file_log = display_info(&v_cfg);

    v_log.info("Starting DroneEngage Storage Server...");
    v_log.info("Initializing database...");
    let v_db_path = match v_cfg.get("database").map(|d| crate::js::prop(Some(d), "path")) {
        Some(Ok(p)) => v_cfg.resolve(&js_interp(p.as_ref())),
        Some(Err(e)) => fail(&v_log, &e),
        None => fail(&v_log, "Cannot read properties of undefined (reading 'path')"),
    };
    let v_db = match Db::open(&v_db_path, v_log.clone()) {
        Ok(db) => db,
        Err(e) => {
            v_log.error(&format!("Database initialization failed: {}", e));
            fail(&v_log, "Database initialization failed")
        }
    };
    let v_storage = Storage::new(
        v_cfg,
        v_db,
        v_log.clone(),
        std::env::var(crate::s2s_auth::S2S_SHARED_SECRET_ENV).ok(),
        v_file_log,
    );

    {
        let v_storage = v_storage.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                check_memory(&v_storage);
            }
        });
    }

    if let Err(e) = crate::start(&v_storage).await {
        v_log.error(&format!("Failed to start storage server: {}", e));
        std::process::exit(1);
    }

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut v_term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                if let Some(l) = &v_storage.file_log {
                    l.warn("SIGINT.", None, None);
                }
                std::process::exit(0);
            }
            _ = v_term.recv() => {
                v_log.info("Received SIGTERM, shutting down gracefully...");
                crate::stop(&v_storage).await;
                std::process::exit(0);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        std::process::exit(0);
    }
}

/// A startup failure: "Failed to start storage server: ..." and exit 1.
fn fail(p_log: &Log, p_error: &str) -> ! {
    p_log.error(&format!("Failed to start storage server: {}", p_error));
    std::process::exit(1);
}

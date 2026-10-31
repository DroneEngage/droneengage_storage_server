# AGENTS.md — droneengage_storage_server

DroneEngage storage server (binary `de_storage_server`). Persistent
network storage for units: task data (LoadTasks / SaveTasks /
DeleteTasks / DisableTasks), unit records and per-unit offline queues.
Comm servers reach it over an S2S-authenticated (Ed25519 challenge-
response) WebSocket. **Rust (tokio)** port of the Node.js server —
protocol, `server.config` and state files unchanged; Node code on
`pr_nodejs_last_version`. See `../../AGENTS.md`.

## Build / test / run

    cargo build --release     # -> target/release/de_storage_server
    cargo test                # unit + integration tests (tests/)

Run from the repo root — `server.config`, the `views/` + `public/`
dashboard dirs, SSL files and SQLite paths are resolved from the cwd.
`../sh_start_servers.sh` starts it with the other servers.

## Config / source

`server.config` + `server.config.local`. `src/` — `main.rs`, `cli.rs`,
`server.rs`, `ws_server.rs` (S2S wss listener), `db.rs` (SQLite),
`handlers.rs` (task operations), `offline_queue.rs`, `dashboard/`
(admin dashboard), `bin/`, `logger.rs`, `config.rs`. `tests/` —
integration tests. `wiki/` — design docs.

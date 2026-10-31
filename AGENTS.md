# AGENTS.md — droneengage_storage_server

DroneEngage storage server (binary `de_storage_server`). Persistent
storage for units: task data, unit records, per-unit offline queues;
reached over an S2S-authenticated WebSocket. **Rust (tokio)** port of
the Node.js server (old code on `pr_nodejs_last_version`).

## Quickstart

    cargo build --release     # -> target/release/de_storage_server
    cargo test                # unit + integration tests

## Index — `.agents/wiki/`

| File | What it covers |
|---|---|
| [`agents_overview.md`](.agents/wiki/agents_overview.md) | run-from-repo-root rule, config, source layout |

See `../../AGENTS.md` for the workspace and shared server conventions.

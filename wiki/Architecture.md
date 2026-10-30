# Architecture

This document describes the internal architecture of the DroneEngage Storage Server.

## Component Overview

The server is composed of five core modules, started in order by `src/lib.rs` (`start`), with the shared state (configuration, database, S2S credentials, connections) in `Storage` (`src/server.rs`).

```
                 ┌─────────────────────────────────────────────┐
                 │              StorageServer                   │
                 │     (src/lib.rs, src/server.rs)              │
                 │                                              │
                 │  ┌──────────────┐   ┌───────────────────┐   │
  Comm Server ──►│  │WebSocketServer│──►│ MessageHandlers   │   │
  (S2S WS)       │  │  + S2S Auth   │   │  (9001-9012)      │   │
                 │  └──────────────┘   └────────┬──────────┘   │
                 │                              │              │
                 │  ┌──────────────┐   ┌────────▼──────────┐   │
                 │  │ OfflineQueue  │   │  DatabaseManager   │   │
                 │  │ (per-unit)    │   │  (rusqlite)        │   │
                 │  └──────────────┘   └────────┬──────────┘   │
                 │                              │              │
                 │                     ┌────────▼──────────┐   │
                 │                     │   SQLite DB file   │   │
                 │                     │  (data/storage.db) │   │
                 │                     └────────────────────┘   │
                 └─────────────────────────────────────────────┘
```

### Startup and shared state (`src/lib.rs`, `src/server.rs`, `src/cli.rs`)

The top-level orchestrator. Responsibilities:

- Opens the database, starts the offline-queue cleanup job, the WebSocket server and the dashboard in the order the Node.js server did.
- Holds the live configuration and the connection table every module uses.
- Prints startup statistics (DB size, unit/task counts, queue depth, connection count).
- Shuts down gracefully on `SIGTERM`; `SIGINT` exits at once.

### WebSocket server (`src/ws_server.rs`)

Accepts S2S WebSocket connections from comm servers. Key behaviors:

- Listens with TLS (rustls) when `enable_SSL` and both certificate files are configured, else plain.
- Keeps the active connections by id.
- Sends every new connection an S2S auth challenge: Ed25519 when `s2s_cert_enabled` is `true`, HMAC-SHA256 with the shared secret otherwise. A connection must answer within 8 s.
- Routes incoming JSON messages to the appropriate handler by numeric `mt` field.
- No server-side ping/pong; the comm server tracks liveness via its own reconnect/heartbeat logic.

### Message handlers (`src/handlers.rs`)

Maps Andruav message types to handlers. Each handler:

1. Extracts the payload from the `ms` field (falls back to the message root for tests).
2. Performs the database operation.
3. Logs access to the `access_log` table.
4. Sends a response envelope that echoes the request `rid` for caller correlation.

Supported types: `9001`-`9004` (tasks), `9009` (unit online), `9010`-`9012` (missions), `9015`-`9017` (news, with `9018` pushes). See [Message Protocol](MessageProtocol.md).

### Database (`src/db.rs`)

Wraps SQLite (rusqlite, bundled) with prepared statements for all CRUD operations. Values are bound the way better-sqlite3 bound JavaScript values, so data written by either server reads back the same. Key design points:

- WAL journal mode and foreign keys enabled on init.
- Tables created with `CREATE TABLE IF NOT EXISTS` (idempotent).
- Migrations add `account_id`, `kind`, `size` and `meta` to `missions` and backfill missing ids and sizes.
- All task/mission data is stored as JSON text and parsed on read.
- Upsert pattern (`INSERT ... ON CONFLICT DO UPDATE`) with automatic `version` increment.

See [Database Schema](DatabaseSchema.md) for full table definitions.

### Offline queue (`src/offline_queue.rs`)

Per-unit message queue for buffering messages while a unit is offline. Key behaviors:

- Enforces a max queue size per unit (`queue.maxQueueSize`).
- Processes queued messages when a `UnitOnline` (9009) message arrives.
- Delivered messages are marked and cleaned up hourly.
- Messages are ordered by `priority DESC, created_at ASC`.

See [Offline Queue](OfflineQueue.md) for details.

## Startup Flow

The startup sequence (`src/cli.rs` → `start()` in `src/lib.rs`):

1. **Parse CLI args**: `--config`, `-h`, `-v`.
2. **Load config**: reads `server.config` (JSON with comments) and merges `server.config.local`.
3. **Display info**: prints server ID, port, auth server endpoint, logging status.
4. **Open the database**: creates tables and indexes, runs the migrations.
5. **Start the offline-queue cleanup job** (hourly).
6. **Start the WebSocket server**: checks the S2S mode (fails closed without a valid shared secret), loads TLS, binds.
7. **Print statistics**: DB size, units, tasks, queue depth, connections.
8. **Start the dashboard** when `dashboard_enable`.
9. **Report the listeners**, then serve until SIGTERM/SIGINT.

A memory check runs every 60 seconds. If RSS exceeds `memory_max` (when configured), the process exits with code 1 (for external restart).

## Shutdown Flow

`StorageServer.stop()` performs graceful shutdown:

1. Closes all WebSocket connections (code 1001, "Server shutting down").
2. Checkpoints and closes the SQLite database.

## File Structure

```
droneengage_storage_server/
├── Cargo.toml
├── server.config              # Default configuration file
├── src/
│   ├── main.rs                # Binary entry point
│   ├── cli.rs                 # Arguments, banner, memory watchdog, signals
│   ├── lib.rs                 # Startup sequence and statistics
│   ├── server.rs              # Shared state and the connection table
│   ├── config.rs              # Config loader (JSON comments, .local overlay, env overrides)
│   ├── ws_server.rs           # WebSocket server + S2S auth challenge
│   ├── handlers.rs            # Message type → handler routing (9001-9017)
│   ├── db.rs                  # SQLite store
│   ├── offline_queue.rs       # Per-unit offline message queue
│   ├── s2s_auth.rs            # Ed25519 / HMAC challenge-response
│   ├── logger.rs              # Console + file logger
│   ├── js.rs, util.rs         # JavaScript-compatible value handling
│   ├── dashboard/             # Admin dashboard (HTTP server, sessions, CSRF, API)
│   └── bin/gen_s2s_keys.rs    # Ed25519 key pair generator
├── views/                     # Dashboard pages (Jinja templates)
├── public/                    # Dashboard static files (Bootstrap)
├── ssl_local/                 # SSL certs + S2S key pairs
│   └── ssl_airgap/            # Air-gap SSL certificates
├── data/                      # SQLite database file (storage.db)
├── logs/                      # Log files (Logs_YYYY-MM-DD.log)
└── tests/                     # cargo integration tests
```

## Integration with Comm Servers

The storage server does not initiate connections. Comm servers connect to it:

1. Comm server reads `storage_server_host` and `storage_server_port` from its config.
2. Comm server opens a WebSocket connection (WSS if SSL enabled).
3. If `s2s_cert_enabled` is `true`, the storage server sends a nonce challenge; the comm server signs it with its Ed25519 private key.
4. Once authenticated, the comm server forwards task/mission operations via its `DBProxyClient`.
5. The comm server reports storage connection status to AUTH for admin visibility.

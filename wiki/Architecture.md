# Architecture

This document describes the internal architecture of the DroneEngage Storage Server.

## Component Overview

The server is composed of five core modules, all wired together by the `StorageServer` orchestrator class in `src/index.js`.

```
                 ┌─────────────────────────────────────────────┐
                 │              StorageServer                   │
                 │           (src/index.js)                    │
                 │                                              │
                 │  ┌──────────────┐   ┌───────────────────┐   │
  Comm Server ──►│  │WebSocketServer│──►│ MessageHandlers   │   │
  (S2S WS)       │  │  + S2S Auth   │   │  (9001-9012)      │   │
                 │  └──────────────┘   └────────┬──────────┘   │
                 │                              │              │
                 │  ┌──────────────┐   ┌────────▼──────────┐   │
                 │  │ OfflineQueue  │   │  DatabaseManager   │   │
                 │  │ (per-unit)    │   │  (better-sqlite3)  │   │
                 │  └──────────────┘   └────────┬──────────┘   │
                 │                              │              │
                 │                     ┌────────▼──────────┐   │
                 │                     │   SQLite DB file   │   │
                 │                     │  (data/storage.db) │   │
                 │                     └────────────────────┘   │
                 └─────────────────────────────────────────────┘
```

### StorageServer (`src/index.js`)

The top-level orchestrator. Responsibilities:

- Initializes the `DatabaseManager`, `MessageHandlers`, `OfflineQueue`, and `WebSocketServer` in the correct order.
- Wires cross-references (e.g. injects `wsServer` into `messageHandlers` and `offlineQueue` after the WS server is created).
- Prints startup statistics (DB size, unit/task counts, queue depth, connection count).
- Handles graceful shutdown on `SIGTERM` / `SIGINT` / `uncaughtException`.

### WebSocketServer (`src/websocketServer.js`)

Accepts S2S WebSocket connections from comm servers. Key behaviors:

- Creates an `https` or `http` server depending on `enable_SSL`.
- Maintains a `Map<connectionId, connectionInfo>` of all active connections.
- On each new connection, either sends an S2S auth challenge (if `s2s_cert_enabled` is `true`) or marks the connection as authenticated immediately.
- Routes incoming JSON messages to the appropriate handler by numeric `mt` field.
- No server-side ping/pong; the comm server tracks liveness via its own reconnect/heartbeat logic.

### MessageHandlers (`src/messageHandlers.js`)

Maps Andruav message types to handler methods via `getHandlers()`. Each handler:

1. Extracts the payload from the `ms` field (falls back to the message root for tests).
2. Performs the database operation.
3. Logs access to the `access_log` table.
4. Sends a response envelope that echoes the request `rid` for caller correlation.

Supported types: `9001`-`9004` (tasks), `9009` (unit online), `9010`-`9012` (missions). See [Message Protocol](MessageProtocol.md).

### DatabaseManager (`src/database.js`)

Wraps `better-sqlite3` with prepared statements for all CRUD operations. Key design points:

- WAL journal mode and foreign keys enabled on init.
- Tables created with `CREATE TABLE IF NOT EXISTS` (idempotent).
- Includes a migration step that adds `account_id` to the `missions` table for backwards compatibility.
- All task/mission data is stored as JSON text and parsed on read.
- Upsert pattern (`INSERT ... ON CONFLICT DO UPDATE`) with automatic `version` increment.

See [Database Schema](DatabaseSchema.md) for full table definitions.

### OfflineQueue (`src/offlineQueue.js`)

Per-unit message queue for buffering messages while a unit is offline. Key behaviors:

- Enforces a max queue size per unit (`queue.maxQueueSize`).
- Processes queued messages when a `UnitOnline` (9009) message arrives.
- Delivered messages are marked and cleaned up hourly.
- Messages are ordered by `priority DESC, created_at ASC`.

See [Offline Queue](OfflineQueue.md) for details.

## Startup Flow

The startup sequence in `server.js` → `StorageServer.start()`:

1. **Parse CLI args** (`fn_parseArgs`): `--config`, `-h`, `-v`.
2. **Load config** (`serverConfig.init`): reads `server.config`, strips JSON comments, parses.
3. **Display info** (`fn_displayInfo`): prints server ID, port, auth server endpoint, logging status.
4. **Initialize database**: `DatabaseManager.initialize()` creates tables and indexes.
5. **Initialize message handlers**: `new MessageHandlers(db, null)` (wsServer injected later).
6. **Initialize offline queue**: `new OfflineQueue(db, null)` + `startCleanupJob()`.
7. **Initialize WebSocket server**: `new WebSocketServer(handlers)` + `start()`.
8. **Wire cross-references**: inject `wsServer` into `messageHandlers` and `offlineQueue`.
9. **Setup shutdown handlers**: SIGTERM, SIGINT, uncaughtException, unhandledRejection.
10. **Print statistics**: DB size, units, tasks, queue depth, connections.

A memory check runs every 60 seconds. If RSS exceeds `memory_max` (when configured), the process exits with code 1 (for external restart).

## Shutdown Flow

`StorageServer.stop()` performs graceful shutdown:

1. Closes all WebSocket connections (code 1001, "Server shutting down").
2. Closes the WebSocket server.
3. Closes the SQLite database connection.

## File Structure

```
droneengage_storage_server/
├── server.js                  # Entry point: arg parsing, config loading, startup
├── js_serverConfig.js         # Config loader (strips JSON comments)
├── server.config              # Default configuration file
├── package.json
├── src/
│   ├── index.js               # StorageServer orchestrator class
│   ├── websocketServer.js     # WebSocket server + S2S auth challenge
│   ├── messageHandlers.js     # Message type → handler routing (9001-9012)
│   ├── database.js            # DatabaseManager (better-sqlite3 wrapper)
│   ├── offlineQueue.js        # Per-unit offline message queue
│   ├── js_s2s_auth.js         # Ed25519 challenge-response auth helper
│   └── logger.js              # Console + file logger wrapper
├── helpers/
│   ├── hlp_args.js            # CLI argument parser
│   ├── hlp_strings.js
│   ├── hlp_validation.js
│   ├── js_3rd_StripJsonComments.js  # JSON comment stripper
│   ├── js_colors.js           # Terminal color codes
│   ├── js_pages.js
│   └── js_styleHelper.js
├── scripts/
│   └── gen_s2s_keys.js        # Ed25519 key pair generator
├── ssl_local/                 # SSL certs + S2S key pairs
│   └── ssl_airgap/            # Air-gap SSL certificates
├── data/                      # SQLite database file (storage.db)
├── logs/                      # Log files (Logs_YYYY-MM-DD.log)
└── tests/
    └── test_mission_storage.js
```

## Integration with Comm Servers

The storage server does not initiate connections. Comm servers connect to it:

1. Comm server reads `storage_server_host` and `storage_server_port` from its config.
2. Comm server opens a WebSocket connection (WSS if SSL enabled).
3. If `s2s_cert_enabled` is `true`, the storage server sends a nonce challenge; the comm server signs it with its Ed25519 private key.
4. Once authenticated, the comm server forwards task/mission operations via its `DBProxyClient`.
5. The comm server reports storage connection status to AUTH for admin visibility.

# DroneEngage Storage Server

Storage server for DroneEngage system - handles task data persistence for units. This server provides permanent network storage that units can use to store their data and load tasks stored by GCS when units were offline.

## Architecture

The storage server implements the following architecture:

- **WebSocket Server**: Accepts S2S WebSocket connections from communication servers with Ed25519 challenge-response handshake
- **Database Persistence**: SQLite database for storing tasks, units, and offline queues
- **Message Handlers**: Handles task messages (LoadTasks, SaveTasks, DeleteTasks, DisableTasks)
- **Offline Queue**: Per-unit offline queue for tasks when units are offline
- **S2S Certificate Authentication**: Validates comm server connections using Ed25519 public keys

## Integration Flow

The storage server integrates with the DroneEngage system as follows:

1. **Comm servers → Storage server (direct WebSocket)**: Comm servers connect directly to storage server using configured endpoint (storage_server_host and storage_server_port in comm server config)
2. **S2S Authentication**: Comm servers authenticate using Ed25519 challenge-response handshake
3. **Task operations**: Comm servers forward LoadTasks/SaveTasks/DeleteTasks/DisableTasks to storage server via DBProxyClient
4. **Status reporting**: Comm servers report storage connection status to AUTH for admin visibility

## Installation

The server is written in Rust. Install a stable toolchain
([rustup](https://rustup.rs)), then build:

```bash
cargo build --release
```

The binary is `target/release/de_storage_server`. Run it from the
repository root: `server.config`, the `views/` and `public/` directories of
the dashboard, the SSL files and relative database paths are found from the
working directory.

## Configuration

Edit `server.config` to configure:

- `server_id`: Server identifier (default: "DE_StorageSrv")
- `server_ip`: Server IP address (default: "::")
- `public_host`: Public host address (default: "127.0.0.1")
- `server_port`: WebSocket server port (default: 9000)
- `enable_SSL`: Enable SSL/TLS for WebSocket connections (default: true) - SSL is independent of S2S cert auth
- `ssl_key_file`: Path to SSL private key file (default: "./ssl_local/ssl_airgap/domain.key")
- `ssl_cert_file`: Path to SSL certificate file (default: "./ssl_local/ssl_airgap/domain.crt")
- `allow_fake_SSL`: Allow self-signed certificates (default: true)
- `ca_cert_path`: Path to CA certificate (default: "./ssl_local/ssl_airgap/root.crt")
- `s2s_cert_enabled`: Enable S2S certificate authentication for comm servers (default: false)
- `s2s_trusted_server_keys`: Map of trusted comm server IDs to their public key paths
- `database.path`: SQLite database file path (default: "./data/storage.db")
- `database.maxConnections`: Maximum SQLite connections (default: 10)
- `queue.maxQueueSize`: Maximum queue size per unit (default: 1000)
- `queue.retryInterval`: Offline queue retry interval in ms (default: 5000)
- `ignoreLog`: Disable logging (default: false)
- `log_directory`: Log files directory (default: "./logs/")
- `log_timeZone`: Timezone for logs (default: "GMT")
- `log_detailed`: Enable debug logging (default: false)

## Running

```bash
./target/release/de_storage_server
```

During development:
```bash
cargo run
```

### Command-line Options

- `--config=<filename>`: Use custom config file (default: server.config)
- `-h, --help`: Display help
- `-v, --version`: Display version

Example:
```bash
./target/release/de_storage_server --config=myconfig.config
```

### S2S keys

```bash
cargo run --release --bin gen_s2s_keys -- DE_CommSrv
```

writes `ssl_local/DE_CommSrv_private.pem` and `ssl_local/DE_CommSrv_public.pem`.

## Testing

```bash
cargo test
```

CI also runs `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.

## Shared code

Configuration loading, JavaScript-compatible JSON handling, the file
logger, TLS, S2S authentication, admin validation and `$$HASH$$` handling
come from
[droneengage_server_common](https://github.com/HefnySco/prv_droneengage_server_common)
(branch `pr_rust`), shared with the comm server and the authenticator.

## Differences from the Node.js server

The Rust server replaces the Node.js one in place: the same `server.config`
(and `.local` overlay), database file and schema, S2S protocol, dashboard
URLs and API, and console output. Behaviour was checked by differential
tests against the Node.js server (protocol, database contents, migration
of old databases, startup failures, the dashboard's pages, headers, cookies
and API). Known differences:

- Dashboard views are Jinja templates (`views/*.html`, rendered with
  minijinja) instead of EJS; the HTML they produce is the same.
- The dashboard's error page (e.g. for a malformed JSON body) shows the
  error without a stack trace, and JSON parse errors are worded
  differently (also in the log).
- Static files ignore `Range` requests (the whole file is sent).
- `log_timeZone` supports UTC only.

## API

### S2S Authentication

Communication servers authenticate using Ed25519 signature-based challenge-response:

1. Storage server sends a random nonce challenge
2. Comm server signs the nonce with its private key
3. Storage server verifies signature using comm server's public key
4. Authentication successful if signature is valid

### Task Messages

Task messages use the Andruav protocol format with `mt` (message type) and `ms` (message data):

#### LoadTasks (9001)
```json
{
  "mt": 9001,
  "ms": {
    "unitId": "unit-id"
  },
  "rid": "request-id"
}
```

#### SaveTasks (9002)
```json
{
  "mt": 9002,
  "ms": {
    "unitId": "unit-id",
    "tasks": [
      {
        "taskId": "task-id",
        "name": "Task Name",
        "data": { ... }
      }
    ]
  },
  "rid": "request-id"
}
```

#### DeleteTasks (9003)
```json
{
  "mt": 9003,
  "ms": {
    "unitId": "unit-id",
    "taskIds": ["task-id-1", "task-id-2"]
  },
  "rid": "request-id"
}
```

#### DisableTasks (9004)
```json
{
  "mt": 9004,
  "ms": {
    "unitId": "unit-id",
    "taskIds": ["task-id-1", "task-id-2"]
  },
  "rid": "request-id"
}
```

#### UnitOnline (9009)
```json
{
  "mt": 9009,
  "ms": {
    "unitId": "unit-id"
  }
}
```

**Note:** The `rid` (request ID) field is used for request-response correlation by the comm server's DBProxyClient.

## Database Schema

### Units
- `id`: Unit ID (primary key)
- `name`: Unit name
- `comm_server_id`: Communication server ID
- `created_at`: Creation timestamp
- `updated_at`: Last update timestamp

### Tasks
- `id`: Task ID (primary key)
- `unit_id`: Unit ID (foreign key)
- `name`: Task name
- `data`: Task data (JSON)
- `version`: Task version
- `created_at`: Creation timestamp
- `updated_at`: Last update timestamp
- `disabled`: Disabled flag

### Offline Queue
- `id`: Queue entry ID (auto-increment)
- `unit_id`: Unit ID (foreign key)
- `task_id`: Task ID (optional)
- `message_type`: Message type
- `message_data`: Message data (JSON)
- `created_at`: Creation timestamp
- `priority`: Priority level
- `status`: Status (pending/delivered)

### Access Log
- `id`: Log entry ID (auto-increment)
- `unit_id`: Unit ID
- `action`: Action performed
- `resource_type`: Resource type
- `resource_id`: Resource ID
- `comm_server_id`: Communication server ID
- `created_at`: Timestamp

## Security

- Ed25519 signature-based authentication for S2S connections
- Comm servers authenticate to storage server using their private keys
- Public keys are configured in `s2s_trusted_server_keys`
- All connections require authentication before processing messages
- SSL/TLS support for encrypted connections

## Monitoring

The server logs to the directory specified in `log_directory` (default: `./logs/`).

Log files are named with date prefix: `Logs_YYYY-MM-DD.log`

Server statistics are displayed on startup:
- Server ID and listening port
- Database statistics (size, units, tasks)
- Active connection count

## License

Proprietary

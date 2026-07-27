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

```bash
npm install
```

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
npm start
```

For development with debugging:
```bash
npm run dev
```

### Command-line Options

- `--config=<filename>`: Use custom config file (default: server.config)
- `-h, --help`: Display help
- `-v, --version`: Display version

Example:
```bash
node server.js --config=myconfig.config
```

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

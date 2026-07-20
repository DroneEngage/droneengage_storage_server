# DroneEngage Storage Server

Storage server for DroneEngage system - handles task data persistence for units. This server provides permanent network storage that units can use to store their data and load tasks stored by GCS when units were offline.

## Architecture

The storage server implements the following architecture:

- **AUTH Registration**: Registers with AUTH server as 'DBServer' type
- **WebSocket Server**: Accepts S2S WebSocket connections from communication servers
- **Session Key Validation**: Validates session keys using asymmetric crypto (AUTH public key) or AUTH API fallback
- **Database Persistence**: SQLite database for storing tasks, units, and offline queues
- **Message Handlers**: Handles task messages (LoadTasks, SaveTasks, DeleteTasks, DisableTasks)
- **Offline Queue**: Per-unit offline queue for tasks when units are offline

## Installation

```bash
npm install
```

## Configuration

Edit `config/default.json` to configure:

- `server.port`: WebSocket server port (default: 9000)
- `auth.endpoint`: AUTH server endpoint
- `auth.heartbeatInterval`: Heartbeat interval in ms (default: 30000)
- `database.path`: SQLite database file path
- `websocket.pingInterval`: WebSocket ping interval (default: 30000)
- `queue.maxQueueSize`: Maximum queue size per unit (default: 1000)

## Running

```bash
npm start
```

For development with debugging:
```bash
npm run dev
```

## API

### Authentication

Communication servers must authenticate using session keys obtained from AUTH:

```json
{
  "type": "auth",
  "sessionKey": "signature.timestamp.commServerId",
  "commServerId": "comm-server-id"
}
```

### Task Messages

#### LoadTasks (9001)
```json
{
  "type": "LoadTasks",
  "unitId": "unit-id"
}
```

#### SaveTasks (9002)
```json
{
  "type": "SaveTasks",
  "unitId": "unit-id",
  "tasks": [
    {
      "taskId": "task-id",
      "name": "Task Name",
      "data": { ... }
    }
  ]
}
```

#### DeleteTasks (9003)
```json
{
  "type": "DeleteTasks",
  "unitId": "unit-id",
  "taskIds": ["task-id-1", "task-id-2"]
}
```

#### DisableTasks (9004)
```json
{
  "type": "DisableTasks",
  "unitId": "unit-id",
  "taskIds": ["task-id-1", "task-id-2"]
}
```

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

- Session keys are validated using AUTH public key (asymmetric crypto)
- Fallback to AUTH API validation if public key is not available
- Session keys are time-limited (4 hours)
- All connections require authentication before processing messages

## Monitoring

The server logs to `logs/combined.log` and `logs/error.log`.

Server statistics can be viewed in the logs on startup.

## License

Proprietary

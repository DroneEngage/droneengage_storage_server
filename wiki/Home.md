# DroneEngage Storage Server - Technical Wiki

The Storage Server is a persistent task and mission storage service for the DroneEngage system. It provides permanent network storage that units (drones/GCS) can use to store their data and load tasks/missions that were saved while they were offline.

## Role in the DroneEngage System

The storage server sits between comm servers and the data layer:

```
  GCS / Units  <-->  Comm Server  <-->  Storage Server  <-->  SQLite DB
```

- Comm servers connect to the storage server via WebSocket (S2S).
- The storage server handles task and mission CRUD operations.
- An offline queue buffers messages for units that are not currently connected.

## Wiki Pages

| Page | Description |
|------|-------------|
| [Architecture](Architecture.md) | Component overview, startup flow, and module structure |
| [Configuration](Configuration.md) | All `server.config` options with defaults and descriptions |
| [Database Schema](DatabaseSchema.md) | SQLite tables, columns, indexes, and migrations |
| [Message Protocol](MessageProtocol.md) | WebSocket message types (9001-9012), request/response formats |
| [S2S Authentication](S2SAuthentication.md) | Ed25519 challenge-response handshake and key management |
| [Offline Queue](OfflineQueue.md) | Per-unit message queue, delivery, and cleanup |
| [Deployment](Deployment.md) | Installation, running, SSL setup, and key generation |

## Quick Start

```bash
npm install
npm start
```

See [Deployment](Deployment.md) for detailed setup instructions.

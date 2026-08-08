# Message Protocol

This document describes the WebSocket message protocol used by the DroneEngage Storage Server.

## Message Format

All messages use the Andruav protocol envelope format with `mt` (message type) and `ms` (message data):

```json
{
  "mt": <numeric message type>,
  "ms": { ... payload ... },
  "rid": "<optional request id>"
}
```

The `rid` (request ID) field is optional. When present in a request, the storage server echoes it back in the response so the comm server's `DBProxyClient` can correlate concurrent requests/responses.

### Response Envelope

All responses follow this structure:

```json
{
  "mt": <same message type>,
  "ms": { "s": "OK" | "ERROR:<message>", ... },
  "success": true | false,
  "timestamp": <Date.now()>,
  "rid": "<echoed from request, if present>",
  "error": "<error message, only on failure>"
}
```

## Message Types

### Task Messages

#### LoadTasks (9001)

Loads all active (non-disabled) tasks for a unit.

**Request:**
```json
{
  "mt": 9001,
  "ms": {
    "unitId": "drone_alpha"
  },
  "rid": "req-001"
}
```

**Response (success):**
```json
{
  "mt": 9001,
  "ms": {
    "s": "OK",
    "tasks": [
      {
        "id": "task-1",
        "unit_id": "drone_alpha",
        "name": "Patrol Route A",
        "data": { ... },
        "version": 3,
        "created_at": 1723000000,
        "updated_at": 1723100000,
        "disabled": 0
      }
    ],
    "unitId": "drone_alpha"
  },
  "success": true,
  "timestamp": 1723100000,
  "rid": "req-001"
}
```

Tasks are returned ordered by `created_at DESC`. Only tasks with `disabled = 0` are returned.

---

#### SaveTasks (9002)

Saves (upserts) one or more tasks for a unit. If the unit does not exist, it is created automatically.

**Request:**
```json
{
  "mt": 9002,
  "ms": {
    "unitId": "drone_alpha",
    "tasks": [
      {
        "taskId": "task-1",
        "name": "Patrol Route A",
        "data": { "waypoints": [...] }
      },
      {
        "taskId": "task-2",
        "name": "Return Home",
        "data": { "waypoints": [...] }
      }
    ]
  },
  "rid": "req-002"
}
```

**Response (success):**
```json
{
  "mt": 9002,
  "ms": {
    "s": "OK:save",
    "savedTasks": ["task-1", "task-2"],
    "unitId": "drone_alpha"
  },
  "success": true,
  "timestamp": 1723100000,
  "rid": "req-002"
}
```

Each task is upserted individually. If a task ID already exists, its data is updated and `version` is incremented.

---

#### DeleteTasks (9003)

Permanently deletes one or more tasks by ID.

**Request:**
```json
{
  "mt": 9003,
  "ms": {
    "unitId": "drone_alpha",
    "taskIds": ["task-1", "task-2"]
  },
  "rid": "req-003"
}
```

**Response (success):**
```json
{
  "mt": 9003,
  "ms": {
    "s": "OK:del",
    "deletedTasks": ["task-1", "task-2"],
    "unitId": "drone_alpha"
  },
  "success": true,
  "timestamp": 1723100000,
  "rid": "req-003"
}
```

---

#### DisableTasks (9004)

Marks tasks as disabled (soft delete). Disabled tasks are not returned by `LoadTasks` but remain in the database.

**Request:**
```json
{
  "mt": 9004,
  "ms": {
    "unitId": "drone_alpha",
    "taskIds": ["task-1"]
  },
  "rid": "req-004"
}
```

**Response (success):**
```json
{
  "mt": 9004,
  "ms": {
    "s": "OK:disable",
    "disabledTasks": ["task-1"],
    "unitId": "drone_alpha"
  },
  "success": true,
  "timestamp": 1723100000,
  "rid": "req-004"
}
```

### Unit Online Message

#### UnitOnline (9009)

Sent by the comm server when a unit comes online. Triggers processing of the unit's offline queue.

**Request:**
```json
{
  "mt": 9009,
  "ms": {
    "unitId": "drone_alpha"
  }
}
```

**Response (success):**
```json
{
  "mt": 9009,
  "ms": {
    "s": "OK",
    "unitId": "drone_alpha",
    "processed": 3,
    "failed": 0
  },
  "success": true,
  "timestamp": 1723100000
}
```

`processed` is the number of queued messages successfully delivered; `failed` is the number that could not be sent. See [Offline Queue](OfflineQueue.md) for details.

### Mission Messages

#### LoadMission (9010)

Loads missions for a unit, scoped by `account_id`. If `missionId` is provided, loads a single mission; otherwise loads all missions for the unit+account.

**Request (all missions):**
```json
{
  "mt": 9010,
  "ms": {
    "unitId": "drone_alpha",
    "accountId": "team_alpha"
  },
  "rid": "req-005"
}
```

**Request (specific mission):**
```json
{
  "mt": 9010,
  "ms": {
    "unitId": "drone_alpha",
    "missionId": "mission_001",
    "accountId": "team_alpha"
  },
  "rid": "req-005"
}
```

**Response (success):**
```json
{
  "mt": 9010,
  "ms": {
    "s": "OK",
    "mission": [ ... array of missions ... ],
    "unitId": "drone_alpha"
  },
  "success": true,
  "timestamp": 1723100000,
  "rid": "req-005"
}
```

When loading a specific mission, `mission` is a single object; when loading all, it is an array.

---

#### SaveMission (9011)

Saves (upserts) a mission. If the unit does not exist, it is created automatically.

**Request:**
```json
{
  "mt": 9011,
  "ms": {
    "unitId": "drone_alpha",
    "missionId": "mission_001",
    "accountId": "team_alpha",
    "name": "Patrol Mission 1",
    "data": {
      "version": 1,
      "mav_waypoints": [ ... ],
      "meta": { "platform": "quad" }
    }
  },
  "rid": "req-006"
}
```

**Response (success):**
```json
{
  "mt": 9011,
  "ms": {
    "s": "OK:save",
    "missionId": "mission_001",
    "unitId": "drone_alpha"
  },
  "success": true,
  "timestamp": 1723100000,
  "rid": "req-006"
}
```

---

#### DeleteMission (9012)

Deletes a mission, scoped by `account_id`.

**Request:**
```json
{
  "mt": 9012,
  "ms": {
    "missionId": "mission_001",
    "accountId": "team_alpha"
  },
  "rid": "req-007"
}
```

**Response (success):**
```json
{
  "mt": 9012,
  "ms": {
    "s": "OK:delete",
    "missionId": "mission_001",
    "deleted": 1
  },
  "success": true,
  "timestamp": 1723100000,
  "rid": "req-007"
}
```

`deleted` is the number of rows affected (0 if the mission did not exist or belonged to a different account).

## Error Responses

All message types return errors in the same format:

```json
{
  "mt": <message type>,
  "ms": {
    "s": "ERROR:<error message>",
    "unitId": "drone_alpha"
  },
  "success": false,
  "timestamp": 1723100000,
  "rid": "<echoed if present>",
  "error": "<error message>"
}
```

## Message Routing

The `WebSocketServer.handleMessage()` method routes messages as follows:

1. **S2S auth check:** If the message is an S2S auth envelope (contains `s2s_auth` field), it is handled as an auth response. No further processing.
2. **Authentication check:** If the connection is not authenticated, an error is sent and the connection is closed (code 1008).
3. **Handler lookup:** The `mt` field is used to look up the handler in the handlers map returned by `MessageHandlers.getHandlers()`.
4. **Unknown type:** If no handler matches, an error is sent: `"Unknown message type: <mt>"`.

## Access Logging

Every successful operation is logged to the `access_log` table with:

| Action | Resource Type | Triggered By |
|--------|--------------|--------------|
| `load` | `task` | LoadTasks (9001) |
| `save` | `task` | SaveTasks (9002) |
| `delete` | `task` | DeleteTasks (9003) |
| `disable` | `task` | DisableTasks (9004) |
| `load` | `mission` | LoadMission (9010) |
| `save` | `mission` | SaveMission (9011) |

The `comm_server_id` from the authenticated connection is recorded with each log entry.

## Message Type Summary

| Type | Name | Description |
|------|------|-------------|
| 9001 | LoadTasks | Load all active tasks for a unit |
| 9002 | SaveTasks | Upsert tasks for a unit |
| 9003 | DeleteTasks | Permanently delete tasks |
| 9004 | DisableTasks | Soft-disable tasks |
| 9009 | UnitOnline | Trigger offline queue processing for a unit |
| 9010 | LoadMission | Load mission(s) for a unit+account |
| 9011 | SaveMission | Upsert a mission |
| 9012 | DeleteMission | Delete a mission |

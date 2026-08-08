# Offline Queue

This document describes the offline queue mechanism in the DroneEngage Storage Server.

## Overview

The offline queue buffers messages for units that are not currently connected to a comm server. When a unit comes back online, the queued messages are delivered to it via the comm server's WebSocket connection.

The queue is implemented in `src/offlineQueue.js` and backed by the `offline_queue` table in SQLite.

## Queue Lifecycle

```
  Unit offline                    Unit comes online
       │                                │
       │  Comm server receives          │  Comm server sends
       │  a task message for            │  UnitOnline (9009)
       │  an offline unit               │
       │                                │
       ▼                                ▼
  ┌──────────┐                    ┌──────────────┐
  │ enqueue  │                    │ processQueue │
  │ message  │                    │ ForUnit()    │
  └────┬─────┘                    └──────┬───────┘
       │                                 │
       ▼                                 ▼
  ┌──────────┐                    ┌──────────────┐
  │ offline_ │                    │ Deliver each │
  │ queue    │                    │ message via  │
  │ (pending)│                    │ WebSocket    │
  └──────────┘                    └──────┬───────┘
                                         │
                                         ▼
                                 ┌──────────────┐
                                 │ Mark as      │
                                 │ 'delivered'  │
                                 └──────┬───────┘
                                        │
                                        ▼
                                 ┌──────────────┐
                                 │ Hourly       │
                                 │ cleanup job  │
                                 │ (>24h old)   │
                                 └──────────────┘
```

## Enqueue

The `enqueue()` method adds a message to a unit's offline queue:

```javascript
offlineQueue.enqueue(unitId, messageType, messageData, taskId, priority)
```

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `unitId` | string | — | Unit to queue the message for |
| `messageType` | number | — | Andruav message type (e.g., 9002 for SaveTasks) |
| `messageData` | object | — | Message payload (stored as JSON) |
| `taskId` | string | `null` | Optional task ID associated with the message |
| `priority` | number | `0` | Priority (higher = delivered first) |

### Queue Size Limit

Each unit's queue is capped at `queue.maxQueueSize` (default: 1000). If the queue is full, `enqueue()` returns:

```json
{ "success": false, "error": "Queue full" }
```

## Processing

Queue processing is triggered by the `UnitOnline` (9009) message. When a comm server reports that a unit is online, the `handleUnitOnline` handler calls `processQueueForUnit()`:

```javascript
const result = await offlineQueue.processQueueForUnit(unitId, connectionId);
// result: { success: true, processed: N, failed: M }
```

### Processing Steps

1. Retrieve up to 50 pending messages for the unit, ordered by `priority DESC, created_at ASC`.
2. For each message, call `deliverMessage()` to send it via the WebSocket connection.
3. If delivery succeeds, mark the message as `delivered` in the database.
4. If delivery fails, increment the failed counter (message remains `pending`).
5. After processing, run `cleanupDeliveredMessages()` to remove old delivered entries.
6. Return counts of processed and failed messages.

### Delivery Format

Delivered messages are sent to the comm server as:

```json
{
  "type": "queued_message",
  "messageType": 9002,
  "data": { ... original message data ... },
  "taskId": "task-1",
  "queueId": 42,
  "timestamp": 1723100000000
}
```

## Cleanup

### Hourly Cleanup Job

`startCleanupJob()` sets up an interval that runs `cleanupDeliveredMessages()` every hour. This deletes delivered messages older than 24 hours:

```sql
DELETE FROM offline_queue
WHERE status = 'delivered' AND created_at < strftime('%s', 'now') - 86400
```

The cleanup job is started during server initialization in `StorageServer.start()`.

### Manual Cleanup

After each `processQueueForUnit()` call, `cleanupDeliveredMessages()` is also invoked immediately to clean up the just-delivered messages.

## Queue Management

### Get Queue Status (per unit)

```javascript
offlineQueue.getQueueStatus(unitId)
// Returns: { total: N, pending: N, delivered: N }
```

### Get Queue Stats (global)

```javascript
offlineQueue.getQueueStats()
// Returns: { totalMessages: N, pendingMessages: N, affectedUnits: N }
```

This is used in the startup statistics output.

### Clear Queue (per unit)

```javascript
offlineQueue.clearQueue(unitId)
// Returns: { success: true, cleared: N }
```

Removes all queued messages (pending and delivered) for a unit.

## Configuration

| Config Key | Default | Description |
|------------|---------|-------------|
| `queue.maxQueueSize` | `1000` | Max pending messages per unit |
| `queue.retryInterval` | `5000` | Retry interval in ms (reserved for future retry logic) |

## Database Schema

See [Database Schema - offline_queue](DatabaseSchema.md#offline_queue) for the full table definition.

| Column | Description |
|--------|-------------|
| `id` | Auto-increment primary key |
| `unit_id` | Target unit (FK → units, CASCADE delete) |
| `task_id` | Optional associated task ID |
| `message_type` | Andruav message type number |
| `message_data` | JSON-encoded message payload |
| `created_at` | Enqueue timestamp (Unix seconds) |
| `priority` | Priority level (higher = first) |
| `status` | `pending` or `delivered` |

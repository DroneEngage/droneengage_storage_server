# Database Schema

This document describes the SQLite database schema used by the DroneEngage Storage Server.

## Overview

The storage server uses [better-sqlite3](https://github.com/WiseLibs/better-sqlite3) as its database engine. The database file is located at `data/storage.db` by default (configurable via `database.path`).

### Initialization

On startup, `DatabaseManager.initialize()` performs the following:

1. Creates the data directory if it does not exist.
2. Opens the database with WAL journal mode and foreign keys enabled.
3. Creates all tables (`CREATE TABLE IF NOT EXISTS`).
4. Runs the `account_id` migration for backwards compatibility.
5. Creates all indexes (`CREATE INDEX IF NOT EXISTS`).

```javascript
this.db.pragma('journal_mode = WAL');
this.db.pragma('foreign_keys = ON');
```

## Tables

### `units`

Stores registered units (drones/GCS).

| Column | Type | Default | Description |
|--------|------|---------|-------------|
| `id` | TEXT | — | Unit ID (primary key) |
| `name` | TEXT | NULL | Unit display name |
| `comm_server_id` | TEXT | NULL | ID of the comm server that registered this unit |
| `created_at` | INTEGER | `strftime('%s', 'now')` | Creation timestamp (Unix epoch seconds) |
| `updated_at` | INTEGER | `strftime('%s', 'now')` | Last update timestamp |

Units are upserted automatically when a `SaveTasks` or `SaveMission` message arrives for a new unit ID.

### `tasks`

Stores task definitions per unit. Task `data` is stored as JSON text.

| Column | Type | Default | Description |
|--------|------|---------|-------------|
| `id` | TEXT | — | Task ID (primary key) |
| `unit_id` | TEXT | — | Unit ID (foreign key → `units.id`, ON DELETE CASCADE) |
| `name` | TEXT | NULL | Task name |
| `data` | TEXT | — | Task data (JSON string, parsed on read) |
| `version` | INTEGER | `1` | Version counter, incremented on each upsert |
| `created_at` | INTEGER | `strftime('%s', 'now')` | Creation timestamp |
| `updated_at` | INTEGER | `strftime('%s', 'now')` | Last update timestamp |
| `disabled` | INTEGER | `0` | Disabled flag (0 = active, 1 = disabled) |

**Upsert behavior:** `INSERT ... ON CONFLICT(id) DO UPDATE SET ... version = version + 1`. This means saving a task with an existing ID updates it in place and increments the version.

**Load behavior:** `loadTasks(unitId)` returns only tasks where `disabled = 0`, ordered by `created_at DESC`.

### `missions`

Stores mission definitions per unit, scoped by `account_id` for multi-tenant isolation.

| Column | Type | Default | Description |
|--------|------|---------|-------------|
| `id` | TEXT | — | Mission ID (primary key) |
| `unit_id` | TEXT | — | Unit ID (foreign key → `units.id`, ON DELETE CASCADE) |
| `account_id` | TEXT | — | Account ID for scoping (multi-tenant isolation) |
| `name` | TEXT | NULL | Mission name |
| `data` | TEXT | — | Mission data (JSON string, parsed on read) |
| `version` | INTEGER | `1` | Version counter, incremented on each upsert |
| `created_at` | INTEGER | `strftime('%s', 'now')` | Creation timestamp |
| `updated_at` | INTEGER | `strftime('%s', 'now')` | Last update timestamp |

**Account scoping:** `loadMissions(unitId, accountId)` and `getMission(missionId, accountId)` filter by `account_id`, so different accounts cannot see each other's missions.

**Special unit:** The `_general_` unit ID is used for missions that are not tied to a specific drone.

### `offline_queue`

Per-unit message queue for buffering messages while a unit is offline.

| Column | Type | Default | Description |
|--------|------|---------|-------------|
| `id` | INTEGER | AUTOINCREMENT | Queue entry ID (primary key) |
| `unit_id` | TEXT | — | Unit ID (foreign key → `units.id`, ON DELETE CASCADE) |
| `task_id` | TEXT | NULL | Optional task ID associated with the message |
| `message_type` | INTEGER | — | Andruav message type (e.g., 9002 for SaveTasks) |
| `message_data` | TEXT | — | Message data (JSON string, parsed on read) |
| `created_at` | INTEGER | `strftime('%s', 'now')` | Enqueue timestamp |
| `priority` | INTEGER | `0` | Priority level (higher = delivered first) |
| `status` | TEXT | `'pending'` | Status: `pending` or `delivered` |

**Ordering:** Messages are retrieved ordered by `priority DESC, created_at ASC`.

**Cleanup:** Delivered messages older than 24 hours are deleted by the hourly cleanup job.

See [Offline Queue](OfflineQueue.md) for the full queue lifecycle.

### `access_log`

Audit log of all data access operations.

| Column | Type | Default | Description |
|--------|------|---------|-------------|
| `id` | INTEGER | AUTOINCREMENT | Log entry ID (primary key) |
| `unit_id` | TEXT | NULL | Unit ID involved in the operation |
| `action` | TEXT | — | Action performed: `load`, `save`, `delete`, `disable` |
| `resource_type` | TEXT | NULL | Resource type: `task` or `mission` |
| `resource_id` | TEXT | NULL | Resource ID(s), comma-separated for batch operations |
| `comm_server_id` | TEXT | NULL | Comm server that initiated the request |
| `created_at` | INTEGER | `strftime('%s', 'now')` | Timestamp |

## Indexes

| Index Name | Table | Column(s) | Purpose |
|------------|-------|-----------|---------|
| `idx_tasks_unit_id` | `tasks` | `unit_id` | Fast task lookup by unit |
| `idx_tasks_disabled` | `tasks` | `disabled` | Filter active vs disabled tasks |
| `idx_offline_queue_unit_id` | `offline_queue` | `unit_id` | Fast queue lookup by unit |
| `idx_offline_queue_status` | `offline_queue` | `status` | Filter pending vs delivered |
| `idx_offline_queue_priority` | `offline_queue` | `priority` | Priority ordering |
| `idx_access_log_unit_id` | `access_log` | `unit_id` | Audit log lookup by unit |
| `idx_access_log_created_at` | `access_log` | `created_at` | Time-based audit queries |
| `idx_missions_unit_id` | `missions` | `unit_id` | Fast mission lookup by unit |
| `idx_missions_account_id` | `missions` | `account_id` | Account-scoped mission queries |

## Migrations

### `account_id` column on `missions`

On startup, the server attempts to add an `account_id` column to the `missions` table:

```sql
ALTER TABLE missions ADD COLUMN account_id TEXT NOT NULL DEFAULT '_unknown_'
```

If the column already exists (error contains "duplicate column"), the migration is silently skipped. This ensures backwards compatibility with databases created before account-scoped missions were introduced.

## Foreign Key Cascade

All child tables (`tasks`, `missions`, `offline_queue`) have `ON DELETE CASCADE` foreign keys to `units.id`. Deleting a unit automatically removes all its tasks, missions, and queued messages.

## Database Statistics

The `getStats()` method returns:

| Field | Description |
|-------|-------------|
| `units` | Total number of registered units |
| `tasks` | Total number of tasks |
| `disabledTasks` | Number of disabled tasks |
| `dbSize` | Database file size in bytes |

These statistics are printed on startup and available via `getStatus()`.

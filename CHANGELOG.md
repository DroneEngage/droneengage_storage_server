# Changelog

Major changes to the DroneEngage storage server, newest first.

## [1.1.3] - 2026-09-12

- Added per-account news and mission quotas, editable from the admin Settings UI.
- News admin: items can be cloned, enabled/disabled, and hard-deleted; expiry is set with separate date and time inputs.
- Missions can now be deleted from the admin UI.

## [1.1.x] - 2026-09

- Added a `debug_logging` config flag to reduce routine log spam.
- Shared config loading and helpers moved to the `droneengage_server_common` npm package.
- Added the News feature: news storage plus an admin dashboard to manage items.
- Added a `DELETE /api/messages` endpoint for access-log maintenance.
- Added the admin dashboard (with CSP and HTML-escaping fixes).
- Added mission storage with account-scoped create/read/update/delete operations.
- Server port can be overridden with an environment variable.

## [1.0.x / 0.x] - earlier

- Initial storage server: stores messages and access logs, serves them to authorized clients.
- Added server-to-server (S2S) key generation and removed manual WebSocket ping/pong.
- Added SSL sample configuration.

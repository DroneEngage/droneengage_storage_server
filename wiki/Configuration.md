# Configuration

This document describes all configuration options for the DroneEngage Storage Server.

## Configuration File

The server is configured via `server.config` (JSON with C-style comments). The comments are stripped at load time by `js_3rd_StripJsonComments.js`, so inline `//` comments are supported.

A custom config file can be specified via CLI:

```bash
node src/server.js --config=myconfig.config
```

The config file path is resolved relative to the project root (`__dirname` of `js_serverConfig.js`).

## Full Default Configuration

```json
{
    // Server configuration
    "server_id"                     : "DE_StorageSrv",
    "server_ip"                     : "::",
    "public_host"                   : "127.0.0.1",
    "server_port"                   : 9000,
    "enable_SSL"                    : true,

    // SSL certificates for WebSocket server
    "ssl_key_file"                  : "./ssl_local/ssl_airgap/domain.key",
    "ssl_cert_file"                 : "./ssl_local/ssl_airgap/domain.crt",
    "allow_fake_SSL"                : true,
    "ca_cert_path"                  : "./ssl_local/ssl_airgap/root.crt",

    // Server-to-Server (S2S) certificate authentication
    "s2s_cert_enabled"              : false,
    "s2s_trusted_server_keys"       : {
        "DE_CommSrv"                : "./ssl_local/DE_ServerComm_public.pem"
    },

    // Database configuration
    "database": {
        "path"                      : "./data/storage.db",
        "maxConnections"            : 10
    },

    // Logging configuration
    "ignoreLog"                     : false,
    "log_directory"                 : "./logs/",
    "log_timeZone"                  : "GMT",
    "log_detailed"                  : false,

    // Queue configuration
    "queue": {
        "maxQueueSize"              : 1000,
        "retryInterval"             : 5000
    }
}
```

## Server Settings

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `server_id` | string | `"DE_StorageSrv"` | Unique server identifier, used in S2S auth and logging |
| `server_ip` | string | `"::"` | IP address to bind to (`::` = all IPv6/IPv4 interfaces) |
| `public_host` | string | `"127.0.0.1"` | Public host address reported in startup info |
| `server_port` | number | `9000` | WebSocket server port. Overridden by `de_storage_server_port` env var if set |
| `enable_SSL` | boolean | `true` | Enable SSL/TLS for WebSocket connections. Independent of S2S cert auth |

### Port Override

The `server_port` can be overridden at runtime via the `de_storage_server_port` environment variable:

```bash
de_storage_server_port=9100 npm start
```

## SSL / TLS Settings

SSL provides transport encryption for the WebSocket connection. It is independent of S2S certificate authentication.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `ssl_key_file` | string | `"./ssl_local/ssl_airgap/domain.key"` | Path to SSL private key (relative to project root) |
| `ssl_cert_file` | string | `"./ssl_local/ssl_airgap/domain.crt"` | Path to SSL certificate (relative to project root) |
| `allow_fake_SSL` | boolean | `true` | Allow self-signed certificates |
| `ca_cert_path` | string | `"./ssl_local/ssl_airgap/root.crt"` | Path to CA certificate |

If `enable_SSL` is `true` but `ssl_cert_file` or `ssl_key_file` are missing, the server falls back to plain HTTP with a warning.

## S2S Authentication Settings

S2S certificate authentication validates the identity of connecting comm servers using Ed25519 signatures. See [S2S Authentication](S2SAuthentication.md) for details.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `s2s_cert_enabled` | boolean | `false` | Enable Ed25519 challenge-response auth for comm server connections |
| `s2s_trusted_server_keys` | object | `{ "DE_CommSrv": "..." }` | Map of trusted comm server IDs to their public key PEM file paths |

When `s2s_cert_enabled` is `false`, all connections are accepted without authentication (suitable for air-gapped / development environments).

## Database Settings

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `database.path` | string | `"./data/storage.db"` | SQLite database file path (relative to project root) |
| `database.maxConnections` | number | `10` | Maximum SQLite connections (reserved for future pooling) |

The data directory is created automatically if it does not exist.

## Logging Settings

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `ignoreLog` | boolean | `false` | Disable file logging entirely (console only) |
| `log_directory` | string | `"./logs/"` | Directory for log files |
| `log_timeZone` | string | `"GMT"` | Timezone for log timestamps |
| `log_detailed` | boolean | `false` | Enable debug-level logging (`logLevel: 'debug'` vs `'prod'`) |

Log files are named with a date prefix: `Logs_YYYY-MM-DD.log`.

## Queue Settings

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `queue.maxQueueSize` | number | `1000` | Maximum pending messages per unit before rejecting new ones |
| `queue.retryInterval` | number | `5000` | Offline queue retry interval in milliseconds |

## Memory Management

An optional `memory_max` setting (in MB) can be added to the config. When RSS memory exceeds this value, the server exits with code 1 for external restart:

```json
{
    "memory_max": 512
}
```

This is checked every 60 seconds by the `checkMemory()` function in `src/server.js`.

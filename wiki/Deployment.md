# Deployment

This document covers installation, running, SSL setup, and key generation for the DroneEngage Storage Server.

## Prerequisites

- Node.js (tested with Node 18+)
- npm
- A SQLite-compatible filesystem for the database file

## Installation

```bash
git clone <repo-url>
cd droneengage_storage_server
npm install
```

### Dependencies

| Package | Purpose |
|---------|---------|
| `ws` | WebSocket server |
| `better-sqlite3` | SQLite database driver |
| `express` | HTTP framework (reserved) |
| `winston` | Logging (reserved) |
| `node-cron` | Scheduled tasks (reserved) |
| `axios` | HTTP client (reserved) |
| `node-file-logger` | File-based logging |

## Running

### Production

```bash
npm start
```

This runs `node server.js`.

### Development (with debugger)

```bash
npm run dev
```

This runs `node --inspect server.js`, enabling the Node.js debugger on port 9229.

### Custom Configuration

```bash
node server.js --config=myconfig.config
```

### Port Override

The WebSocket port can be overridden via environment variable without modifying the config file:

```bash
de_storage_server_port=9100 npm start
```

## Command-Line Options

| Option | Description |
|--------|-------------|
| `--config=<filename>` | Use a custom config file (default: `server.config`) |
| `-h`, `--help` | Display help and exit |
| `-v`, `--version` | Display version and exit |

## SSL / TLS Setup

### Using the Provided Air-Gap Certificates

The server ships with self-signed certificates in `ssl_local/ssl_airgap/`:

```
ssl_local/ssl_airgap/
├── domain.crt    # Self-signed certificate
├── domain.key    # Private key
└── root.crt      # CA certificate
```

These are suitable for air-gapped or development environments. For production, replace them with properly signed certificates.

### Generating Custom SSL Certificates

For production deployments, use certificates signed by a trusted CA. Place the key and cert files in `ssl_local/` and update `server.config`:

```json
{
    "enable_SSL": true,
    "ssl_key_file": "./ssl_local/mydomain.key",
    "ssl_cert_file": "./ssl_local/mydomain.crt",
    "ca_cert_path": "./ssl_local/myca.crt",
    "allow_fake_SSL": false
}
```

### Disabling SSL

For environments where transport encryption is handled elsewhere (e.g., a reverse proxy):

```json
{
    "enable_SSL": false
}
```

## S2S Key Generation

Generate an Ed25519 key pair for S2S authentication:

```bash
node scripts/gen_s2s_keys.js DE_CommSrv
```

This creates:

| File | Description |
|------|-------------|
| `ssl_local/DE_CommSrv_private.pem` | Private key (mode 0600) — place on the comm server |
| `ssl_local/DE_CommSrv_public.pem` | Public key — place on the storage server |

Then configure the storage server:

```json
{
    "s2s_cert_enabled": true,
    "s2s_trusted_server_keys": {
        "DE_CommSrv": "./ssl_local/DE_CommSrv_public.pem"
    }
}
```

See [S2S Authentication](S2SAuthentication.md) for the full handshake details.

## Directory Structure After Setup

```
droneengage_storage_server/
├── data/
│   └── storage.db          # SQLite database (auto-created)
├── logs/
│   └── Logs_YYYY-MM-DD.log # Daily log files (auto-created)
├── ssl_local/
│   ├── ssl_airgap/         # Default SSL certificates
│   ├── DE_CommSrv_public.pem   # Comm server public key (for S2S)
│   └── DE_StorageSrv_private.pem  # Storage server private key (if connecting out)
└── server.config           # Configuration file
```

## Logging

### Log Output

Logs are written to both console and file (unless `ignoreLog` is `true`):

- **Console:** Colorized output with `[INFO]`, `[ERROR]`, `[WARN]`, `[DEBUG]` prefixes.
- **File:** `Logs_YYYY-MM-DD.log` in the `log_directory` (default: `./logs/`).

### Log Levels

| `log_detailed` | Log Level | Output |
|----------------|-----------|--------|
| `false` | `prod` | Info, Warn, Error |
| `true` | `debug` | Info, Warn, Error, Debug |

### Disabling File Logging

```json
{
    "ignoreLog": true
}
```

When file logging is disabled, only console output is produced.

## Memory Management

An optional `memory_max` setting (in MB) can be added to `server.config`:

```json
{
    "memory_max": 512
}
```

The server checks RSS memory every 60 seconds. If it exceeds `memory_max`, the process exits with code 1. This is designed for use with process managers (systemd, PM2, Docker) that automatically restart the process.

## Process Management

### systemd

Example service file:

```ini
[Unit]
Description=DroneEngage Storage Server
After=network.target

[Service]
Type=simple
User=droneengage
WorkingDirectory=/opt/droneengage/droneengage_storage_server
ExecStart=/usr/bin/node server.js
Restart=always
RestartSec=5
Environment=de_storage_server_port=9000

[Install]
WantedBy=multi-user.target
```

### Docker

```bash
docker build -t droneengage-storage-server .
docker run -d \
  -p 9000:9000 \
  -v $(pwd)/data:/app/data \
  -v $(pwd)/logs:/app/logs \
  -v $(pwd)/ssl_local:/app/ssl_local \
  -v $(pwd)/server.config:/app/server.config \
  droneengage-storage-server
```

> **Note:** Mount `data/`, `logs/`, `ssl_local/`, and `server.config` as volumes to persist state across container restarts.

## Testing

### Mission Storage Tests

```bash
node tests/test_mission_storage.js
```

This runs an in-memory SQLite test suite that validates mission save/load/delete operations, account scoping, version incrementing, and cascade deletes. No real database file is touched.

## Verification Checklist

After deployment, verify:

1. **Server starts:** Check console output for "Storage server started successfully".
2. **Database created:** Verify `data/storage.db` exists.
3. **Port listening:** Verify the WebSocket port is open (`netstat -tlnp | grep 9000`).
4. **SSL working:** Connect with `wss://` and verify no certificate errors.
5. **S2S auth (if enabled):** Verify comm server can authenticate.
6. **Logs writing:** Check `logs/` for daily log files.
7. **Stats output:** Verify startup stats show correct unit/task counts.

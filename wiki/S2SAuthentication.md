# S2S Authentication

This document describes the Server-to-Server (S2S) authentication mechanism used by the DroneEngage Storage Server.

## Overview

The storage server authenticates connecting comm servers using an Ed25519 challenge-response handshake. This ensures that only trusted comm servers can perform task and mission operations.

The implementation is in `src/js_s2s_auth.js` and uses only Node.js built-in `crypto` — no third-party dependencies.

## Handshake Flow

```
  Storage Server (accepts)              Comm Server (connects)
  Holds: PUBLIC keys                    Holds: PRIVATE key
        │                                      │
        │  1. Comm server opens WebSocket       │
        │◄─────────────────────────────────────│
        │                                      │
        │  2. Storage sends nonce challenge     │
        │──────────────────────────────────────►│
        │  { s2s_auth: "challenge", nonce: ... }│
        │                                      │
        │  3. Comm signs nonce with private key │
        │◄──────────────────────────────────────│
        │  { s2s_auth: "response", sig: ..., id: "DE_CommSrv" }
        │                                      │
        │  4. Storage verifies signature        │
        │     using comm's public key           │
        │                                      │
        │  5. Auth success or connection close  │
        │──────────────────────────────────────►│
        │  { type: "auth_success", ... }        │
```

### Step-by-step

1. **Connection:** The comm server opens a WebSocket connection to the storage server.
2. **Challenge:** The storage server generates a 32-byte random nonce (hex-encoded) and sends it as a challenge envelope.
3. **Response:** The comm server signs the nonce with its Ed25519 private key and sends back the base64 signature along with its server ID.
4. **Verification:** The storage server looks up the comm server's public key by ID from `s2s_trusted_server_keys` and verifies the signature.
5. **Result:** If valid, the connection is marked authenticated and an `auth_success` message is sent. If invalid, the connection is closed with code 1008.

## Timeout

The handshake must complete within `CONST_S2S_AUTH_HANDSHAKE_TIMEOUT` (8000 ms). If the comm server does not respond in time, the storage server closes the connection with code 1008 ("Auth timeout").

## Disabling Authentication

When `s2s_cert_enabled` is `false` in the config, the storage server skips the challenge-response handshake entirely and marks all connections as authenticated immediately. This is suitable for:

- Air-gapped / isolated network environments
- Development and testing
- Environments where network-level security is sufficient

## Configuration

### Storage Server (Accepting Side)

The storage server needs the **public keys** of all trusted comm servers:

```json
{
    "s2s_cert_enabled": true,
    "s2s_trusted_server_keys": {
        "DE_CommSrv": "./ssl_local/DE_ServerComm_public.pem",
        "DE_CommSrv2": "./ssl_local/DE_CommSrv2_public.pem"
    }
}
```

| Config Key | Description |
|------------|-------------|
| `s2s_cert_enabled` | Set to `true` to require S2S auth on incoming connections |
| `s2s_trusted_server_keys` | Map of server ID → public key PEM file path |

### Comm Server (Connecting Side)

The comm server needs its own **private key** to sign challenges:

```json
{
    "s2s_my_private_key": "./ssl_local/DE_ServerComm_private.pem"
}
```

The comm server's server ID must match a key in the storage server's `s2s_trusted_server_keys`.

## Key Generation

Use the provided script to generate an Ed25519 key pair:

```bash
node scripts/gen_s2s_keys.js DE_CommSrv
```

This generates two files in `ssl_local/`:

| File | Type | Distribution |
|------|------|-------------|
| `DE_CommSrv_private.pem` | Private key | Copy to the comm server's `ssl_local/` directory. Keep secret. |
| `DE_CommSrv_public.pem` | Public key | Copy to the storage server's `ssl_local/` directory. |

The private key file is written with restrictive permissions (`0o600`).

### Key Distribution Workflow

1. Generate keys: `node scripts/gen_s2s_keys.js DE_CommSrv`
2. Copy `DE_CommSrv_private.pem` to the comm server.
3. Copy `DE_CommSrv_public.pem` to the storage server.
4. Add to storage server config:
   ```json
   "s2s_trusted_server_keys": {
       "DE_CommSrv": "./ssl_local/DE_CommSrv_public.pem"
   }
   ```
5. Add to comm server config:
   ```json
   "s2s_my_private_key": "./ssl_local/DE_CommSrv_private.pem"
   ```

## Envelope Format

### Challenge (storage → comm)

```json
{
  "s2s_auth": "challenge",
  "nonce": "a1b2c3d4e5f6...64 hex chars"
}
```

### Response (comm → storage)

```json
{
  "s2s_auth": "response",
  "sig": "base64-encoded Ed25519 signature",
  "id": "DE_CommSrv"
}
```

### Auth Success (storage → comm)

```json
{
  "type": "auth_success",
  "connectionId": 1,
  "timestamp": 1723100000000
}
```

## Implementation Details

### `js_s2s_auth.js` API

| Function | Description |
|----------|-------------|
| `fn_generateNonce()` | Generates a 32-byte random nonce (hex string) |
| `fn_sign(nonce)` | Signs a nonce with the private key (base64 signature) |
| `fn_verify(nonce, signature, serverId)` | Verifies a signature against a server's public key |
| `fn_buildChallenge(nonce)` | Builds the challenge envelope JSON string |
| `fn_buildResponse(nonce, id)` | Builds the signed response envelope JSON string |
| `fn_parseEnvelope(msg)` | Parses a WS message as an S2S auth envelope; returns `null` if not auth-related |
| `fn_getPrivateKey()` | Lazy-loads and caches the Ed25519 private key |
| `fn_getPublicKeys()` | Lazy-loads and caches all trusted public keys |
| `fn_resetKeyCache()` | Clears key caches (used by tests) |

### Key Caching

Public and private keys are loaded lazily on first use and cached for the lifetime of the process. The `fn_resetKeyCache()` function clears these caches, which is useful in tests when the global config changes between test cases.

### Path Resolution

Key file paths in the config are resolved relative to the project root (one level up from the `src/` directory), not relative to the `js_s2s_auth.js` module itself.

## SSL vs S2S Auth

These are independent security layers:

| Layer | Config | Purpose |
|-------|--------|---------|
| SSL/TLS | `enable_SSL` | Transport encryption (encrypts the WebSocket channel) |
| S2S Cert Auth | `s2s_cert_enabled` | Identity verification (proves the comm server is trusted) |

Both can be enabled independently. For maximum security, enable both. For air-gapped environments, SSL may be sufficient with self-signed certificates.

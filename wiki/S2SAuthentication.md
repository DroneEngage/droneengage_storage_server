# S2S Authentication

This document describes the Server-to-Server (S2S) authentication mechanism used by the DroneEngage Storage Server.

## Overview

The storage server authenticates connecting comm servers using a challenge-response handshake. This ensures that only trusted comm servers can perform task and mission operations. **Every connection must authenticate** — there is no unauthenticated path, and there is no localhost exemption.

Two modes exist, selected by `s2s_cert_enabled`:

| Mode | Config | Credential |
|------|--------|-----------|
| Certificate | `s2s_cert_enabled: true` | Ed25519 signature over the nonce, verified against `s2s_trusted_server_keys` |
| Shared secret | `s2s_cert_enabled: false` | `HMAC-SHA256(nonce, shared_secret)` verified with `crypto.timingSafeEqual` |

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
        │  { s2s_auth: "challenge", nonce: ..., │
        │    method: "ed25519"|"hmac-sha256" }  │
        │                                      │
        │  3. Comm answers the challenge:       │
        │     ed25519 -> sign nonce (priv key)  │
        │     hmac    -> HMAC-SHA256(nonce,     │
        │                shared_secret)         │
        │◄──────────────────────────────────────│
        │  { s2s_auth: "response", sig: ..., id: "DE_CommSrv" }
        │                                      │
        │  4. Storage verifies per its mode:    │
        │     ed25519 -> signature vs public key│
        │     hmac    -> timingSafeEqual(HMAC)  │
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

## Shared-Secret Mode (`s2s_cert_enabled: false`)

**Breaking change:** `s2s_cert_enabled: false` no longer means "no auth". It now selects shared-secret mode — every connection must answer the nonce challenge with `HMAC-SHA256(nonce, shared_secret)`. Deployments that relied on the old open behaviour must provision the same secret on both sides.

The secret is never stored in `server.config` (a shipped value would be a public credential). It is loaded from, in order:

1. The `de_s2s_shared_secret` environment variable.
2. The file named by `s2s_shared_secret_file` (default `./s2s_shared_secret.key`). On the storage server a missing file is **auto-generated** with a random 32-byte hex secret and written `chmod 600` on first boot.

Fail-closed rules: a missing source, a placeholder value (e.g. `CHANGE_ME`), or a secret shorter than 16 chars makes the storage server **refuse to start** with a clear error. The comm server side does *not* auto-generate — a locally generated secret could never match; if the file is absent the handshake fails and the connection closes.

| Config Key / Env | Side | Description |
|---|---|---|
| `s2s_shared_secret_file` | both | Path to the chmod-600 secret file (resolved from the project root) |
| `de_s2s_shared_secret` | both | Env var holding the secret directly (overrides the file) |

The secret is never sent on the wire — only the HMAC of the per-connection nonce.

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
| `s2s_cert_enabled` | `true` = Ed25519 cert mode, `false` = shared-secret mode |
| `s2s_trusted_server_keys` | Map of server ID → public key PEM file path (cert mode) |
| `s2s_shared_secret_file` | Path to the shared-secret file (shared-secret mode) |

Note: with `s2s_cert_enabled=true` and an **empty** `s2s_trusted_server_keys`, every connection is rejected — the server logs a loud boot warning because that configuration is a silent lockout.

### Comm Server (Connecting Side)

The comm server needs its own **private key** to sign challenges (cert mode), or the same **shared secret** (shared-secret mode):

```json
{
    "s2s_my_private_key": "./ssl_local/DE_ServerComm_private.pem",
    "s2s_shared_secret_file": "./s2s_shared_secret.key"
}
```

The challenge envelope carries a `method` field (`ed25519` or `hmac-sha256`) telling the comm server which credential to answer with. Older storage servers send no `method`, which is treated as `ed25519`. A comm server that receives no challenge within `CONST_S2S_AUTH_HANDSHAKE_TIMEOUT` now **fails closed** and drops the connection — the old "no challenge → assume no auth required" fallthrough has been removed.

The comm server's server ID must match a key in the storage server's `s2s_trusted_server_keys` (cert mode).

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
  "nonce": "a1b2c3d4e5f6...64 hex chars",
  "method": "ed25519 | hmac-sha256"
}
```

### Response (comm → storage)

```json
{
  "s2s_auth": "response",
  "sig": "base64 Ed25519 signature OR base64 HMAC-SHA256 of the nonce",
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
| `fn_hmacSign(nonce)` | Computes `HMAC-SHA256(nonce, shared_secret)` (base64) |
| `fn_verifyHmac(nonce, hmac)` | Constant-time verification of an HMAC response |
| `fn_getSharedSecret(autoGenerate)` | Loads the shared secret from env/file; optionally auto-generates the file; throws on missing/placeholder |
| `fn_hasSharedSecret()` | True when a secret source is configured (non-throwing) |
| `fn_buildChallenge(nonce, method)` | Builds the challenge envelope JSON string |
| `fn_buildResponse(nonce, id)` | Builds the Ed25519-signed response envelope JSON string |
| `fn_buildHmacResponse(nonce, id)` | Builds the HMAC response envelope JSON string |
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

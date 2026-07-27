#!/usr/bin/env node

/**
 * Generates an Ed25519 key pair used for Server-to-Server (S2S) authentication.
 *
 * Usage:
 *   node scripts/gen_s2s_keys.js <server_id>
 *
 * Example:
 *   node scripts/gen_s2s_keys.js DE_StorageSrv
 *
 * Output (written to ssl_local/ directory):
 *   <server_id>_private.pem   - PRIVATE key. Keep secret. Required by the server
 *                               that CONNECTS out (storage server to AUTH).
 *   <server_id>_public.pem    - PUBLIC key. Required by servers that ACCEPT peers
 *                               (AUTH server, comm servers).
 *
 * Distribution:
 *   - Copy <server_id>_private.pem to the server's ssl_local directory
 *   - Copy <server_id>_public.pem to the accepting server's ssl_local directory
 *   - Add the public key to the accepting server's s2s_trusted_server_keys config:
 *     "s2s_trusted_server_keys": { "<server_id>": "./ssl_local/<server_id>_public.pem" }
 */

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

function generateS2SKeys(serverId) {
  if (!serverId) {
    console.error('Error: server_id parameter is required');
    console.error('Usage: node scripts/gen_s2s_keys.js <server_id>');
    console.error('Example: node scripts/gen_s2s_keys.js DE_StorageSrv');
    process.exit(1);
  }

  const outDir = path.join(__dirname, '..', 'ssl_local');
  const privateKeyFile = path.join(outDir, `${serverId}_private.pem`);
  const publicKeyFile = path.join(outDir, `${serverId}_public.pem`);

  // Create output directory if it doesn't exist
  if (!fs.existsSync(outDir)) {
    fs.mkdirSync(outDir, { recursive: true });
  }

  // Generate Ed25519 key pair
  const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519');

  // Export keys in PEM format
  const privateKeyPem = privateKey.export({ type: 'pkcs8', format: 'pem' });
  const publicKeyPem = publicKey.export({ type: 'spki', format: 'pem' });

  // Write private key with restrictive permissions
  fs.writeFileSync(privateKeyFile, privateKeyPem, { mode: 0o600 });
  fs.writeFileSync(publicKeyFile, publicKeyPem);

  console.log(`S2S Ed25519 key pair generated for server '${serverId}':`);
  console.log(`  private : ${privateKeyFile}`);
  console.log(`  public  : ${publicKeyFile}`);
  console.log('');
  console.log('Distribution:');
  console.log(`  1. Copy ${privateKeyFile} to the server's ssl_local directory`);
  console.log(`  2. Copy ${publicKeyFile} to the accepting server's ssl_local directory`);
  console.log(`  3. Add the public key to the accepting server's s2s_trusted_server_keys config:`);
  console.log(`     "s2s_trusted_server_keys": { "${serverId}": "./ssl_local/${serverId}_public.pem" }`);
  console.log('');
  console.log('For storage server configuration:');
  console.log(`  "s2s_my_private_key": "./ssl_local/${serverId}_private.pem"`);
}

// Get server_id from command line argument
const serverId = process.argv[2];
generateS2SKeys(serverId);

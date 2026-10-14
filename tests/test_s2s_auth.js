"use strict";

/**
 * Tests for the S2S shared-secret (HMAC) mode in src/js_s2s_auth.js.
 * Covers the s2s_cert_enabled=false path: challenge/response roundtrip,
 * wrong-secret rejection, fail-closed on missing/placeholder secrets,
 * and auto-generation of the secret file.
 *
 * Run:  node tests/test_s2s_auth.js
 */

const assert = require('assert');
const crypto = require('crypto');
const fs = require('fs');
const os = require('os');
const path = require('path');

// Minimal globals the module expects.
global.Colors = { Error: '', Reset: '', FgYellow: '', BSuccess: '' };
global.m_serverconfig = { m_configuration: {} };

const s2sAuth = require('../src/js_s2s_auth.js');

const ENV = s2sAuth.CONST_S2S_SHARED_SECRET_ENV;
let tempDir = null;

function reset() {
    delete process.env[ENV];
    s2sAuth.fn_resetKeyCache();
    global.m_serverconfig.m_configuration = {};
}

function test(name, fn) {
    try {
        reset();
        fn();
        console.log(`  PASS ${name}`);
    } catch (e) {
        console.error(`  FAIL ${name}: ${e.message}`);
        process.exitCode = 1;
    }
}

console.log('=== S2S shared-secret (HMAC) auth tests ===');

test('hmac sign/verify roundtrip via env var', () => {
    process.env[ENV] = 'test-shared-secret-0123456789';
    const nonce = s2sAuth.fn_generateNonce();
    const sig = s2sAuth.fn_hmacSign(nonce);
    assert.strictEqual(s2sAuth.fn_verifyHmac(nonce, sig), true);
});

test('wrong secret is rejected', () => {
    process.env[ENV] = 'test-shared-secret-0123456789';
    const nonce = s2sAuth.fn_generateNonce();
    const wrong = crypto.createHmac('sha256', 'a-different-secret-9876543210').update(nonce).digest('base64');
    assert.strictEqual(s2sAuth.fn_verifyHmac(nonce, wrong), false);
});

test('tampered/wrong-length hmac is rejected', () => {
    process.env[ENV] = 'test-shared-secret-0123456789';
    const nonce = s2sAuth.fn_generateNonce();
    assert.strictEqual(s2sAuth.fn_verifyHmac(nonce, 'AAAA'), false);
    assert.strictEqual(s2sAuth.fn_verifyHmac(nonce, null), false);
    assert.strictEqual(s2sAuth.fn_verifyHmac(null, 'AAAA'), false);
});

test('missing secret config throws (fail closed)', () => {
    assert.throws(() => s2sAuth.fn_getSharedSecret(false), /no secret is configured/);
});

test('placeholder env secret throws (fail closed)', () => {
    process.env[ENV] = 'CHANGE_ME';
    assert.throws(() => s2sAuth.fn_getSharedSecret(false), /placeholder/);
});

test('short secret throws (fail closed)', () => {
    process.env[ENV] = 'tiny';
    assert.throws(() => s2sAuth.fn_getSharedSecret(false), /too short/);
});

test('missing secret file without auto-generate throws', () => {
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 's2s-test-'));
    global.m_serverconfig.m_configuration.s2s_shared_secret_file =
        path.join(tempDir, 'no_such.key');
    assert.throws(() => s2sAuth.fn_getSharedSecret(false), /not found/);
});

test('auto-generate creates a chmod-600 secret file', () => {
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 's2s-test-'));
    const keyPath = path.join(tempDir, 's2s_shared_secret.key');
    global.m_serverconfig.m_configuration.s2s_shared_secret_file = keyPath;
    const secret = s2sAuth.fn_getSharedSecret(true);
    assert.ok(secret.length >= 16);
    const stat = fs.statSync(keyPath);
    assert.strictEqual(stat.mode & 0o777, 0o600);
    // second load reads the same secret back
    s2sAuth.fn_resetKeyCache();
    assert.strictEqual(s2sAuth.fn_getSharedSecret(true), secret);
});

test('challenge envelope carries the requested method', () => {
    const env = JSON.parse(s2sAuth.fn_buildChallenge('nonce123', s2sAuth.CONST_S2S_AUTH_METHOD_HMAC));
    assert.strictEqual(env.s2s_auth, 'challenge');
    assert.strictEqual(env.nonce, 'nonce123');
    assert.strictEqual(env.method, 'hmac-sha256');
    // no method field when omitted (backward compatible)
    const legacy = JSON.parse(s2sAuth.fn_buildChallenge('nonce123'));
    assert.strictEqual('method' in legacy, false);
});

test('hmac response envelope parses and verifies', () => {
    process.env[ENV] = 'test-shared-secret-0123456789';
    const nonce = s2sAuth.fn_generateNonce();
    const env = s2sAuth.fn_parseEnvelope(s2sAuth.fn_buildHmacResponse(nonce, 'DE_CommSrv'));
    assert.strictEqual(env.s2s_auth, 'response');
    assert.strictEqual(env.id, 'DE_CommSrv');
    assert.strictEqual(s2sAuth.fn_verifyHmac(nonce, env.sig), true);
});

if (tempDir) fs.rmSync(tempDir, { recursive: true, force: true });
console.log('=== done ===');

"use strict";

/**
 * Tests for per-account news/mission quotas (TASK-01).
 *
 * Exercises the full stack: real DatabaseManager (temp DB), real
 * MessageHandlers, real config loader (with limits overridden to small
 * values), and a mock wsServer that records sent envelopes.
 *
 * Run:  node tests/test_quota_limits.js
 */

const assert = require('assert');
const fs = require('fs');
const os = require('os');
const path = require('path');

const serverConfig = require('../js_serverConfig.js');
const DatabaseManager = require('../src/database');
const MessageHandlers = require('../src/messageHandlers');

// --- Load real config, then override for the test -------------------------
serverConfig.init();
const cfg = serverConfig.m_configuration;

// Small quotas for the test
cfg.limits = {
    max_news_per_account: 2,
    max_missions_per_account: 2,
    max_missions_per_unit: 1
};

// Temp DB file (DatabaseManager resolves path relative to its own dir, but
// an absolute path wins).
const tmpDb = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'de-quota-')), 't.db');
cfg.database.path = tmpDb;

// --- Mock wsServer --------------------------------------------------------
const sent = [];
const broadcast = [];
const mockWsServer = {
    send: (connId, envelope) => sent.push(envelope),
    broadcast: (envelope) => broadcast.push(envelope),
    getConnection: () => ({ commServerId: 'comm_test' })
};

// --- Setup ----------------------------------------------------------------
const db = new DatabaseManager();
db.initialize();
const handlers = new MessageHandlers(db, mockWsServer);

let testsPassed = 0;
let testsFailed = 0;
function test(name, fn) {
    return Promise.resolve(fn()).then(
        () => { console.log(`  \u2713 ${name}`); testsPassed++; },
        (e) => { console.error(`  \u2717 ${name}\n    ${e.message}`); testsFailed++; }
    );
}

const last = () => sent[sent.length - 1];

// --- Tests ----------------------------------------------------------------
(async () => {
    console.log('\n=== Quota Limit Tests ===\n');

    console.log('--- News quota (max_news_per_account = 2) ---');

    await test('first account news insert succeeds', async () => {
        sent.length = 0;
        await handlers.handleSaveNews('c1', { mt: 9016, ms: { newsId: 'n1', scope: 'account', accountId: 'acct1', body: 'b1' } });
        assert.strictEqual(last().ms.s, 'OK:save');
    });

    await test('second account news insert succeeds', async () => {
        sent.length = 0;
        await handlers.handleSaveNews('c1', { mt: 9016, ms: { newsId: 'n2', scope: 'account', accountId: 'acct1', body: 'b2' } });
        assert.strictEqual(last().ms.s, 'OK:save');
    });

    await test('third account news insert rejected with quota exceeded', async () => {
        sent.length = 0;
        await handlers.handleSaveNews('c1', { mt: 9016, ms: { newsId: 'n3', scope: 'account', accountId: 'acct1', body: 'b3' } });
        assert.ok(last().ms.s.startsWith('ERROR:news quota exceeded'), `got: ${last().ms.s}`);
        assert.strictEqual(last().success, false);
        assert.strictEqual(db.countNewsByAccount('acct1'), 2, 'row count must stay at 2');
    });

    await test('update of existing newsId succeeds (not quota-gated)', async () => {
        sent.length = 0;
        await handlers.handleSaveNews('c1', { mt: 9016, ms: { newsId: 'n1', scope: 'account', accountId: 'acct1', body: 'b1-updated' } });
        assert.strictEqual(last().ms.s, 'OK:save');
        assert.strictEqual(db.countNewsByAccount('acct1'), 2, 'row count must stay at 2');
    });

    await test('global news is exempt from account quota', async () => {
        sent.length = 0;
        await handlers.handleSaveNews('c1', { mt: 9016, ms: { newsId: 'g1', scope: 'global', accountId: null, body: 'gb' } });
        assert.strictEqual(last().ms.s, 'OK:save');
        assert.strictEqual(db.countNewsByAccount('acct1'), 2, 'account count unchanged');
    });

    await test('disabled news frees quota (countNewsByAccount counts disabled=0 only)', async () => {
        db.disableNews('n2');
        assert.strictEqual(db.countNewsByAccount('acct1'), 1);
        sent.length = 0;
        await handlers.handleSaveNews('c1', { mt: 9016, ms: { newsId: 'n4', scope: 'account', accountId: 'acct1', body: 'b4' } });
        assert.strictEqual(last().ms.s, 'OK:save');
    });

    console.log('\n--- Mission quota (per_account = 2, per_unit = 1) ---');

    await test('first mission for unitA succeeds', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitA', missionId: 'm1', accountId: 'acct2', name: 'M1', data: {} } });
        assert.strictEqual(last().ms.s, 'OK:save');
    });

    await test('second mission for same unit rejected by per-unit cap', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitA', missionId: 'm2', accountId: 'acct2', name: 'M2', data: {} } });
        assert.ok(last().ms.s.startsWith('ERROR:mission quota exceeded'), `got: ${last().ms.s}`);
        assert.ok(last().ms.s.includes('per unit'), `got: ${last().ms.s}`);
        assert.strictEqual(last().success, false);
    });

    await test('mission for a second unit succeeds (per-unit resets)', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitB', missionId: 'm3', accountId: 'acct2', name: 'M3', data: {} } });
        assert.strictEqual(last().ms.s, 'OK:save');
        assert.strictEqual(db.countMissionsByAccount('acct2'), 2);
    });

    await test('third mission (any unit) rejected by per-account cap', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitC', missionId: 'm4', accountId: 'acct2', name: 'M4', data: {} } });
        assert.ok(last().ms.s.startsWith('ERROR:mission quota exceeded'), `got: ${last().ms.s}`);
        assert.ok(last().ms.s.includes('per account'), `got: ${last().ms.s}`);
        assert.strictEqual(db.countMissionsByAccount('acct2'), 2);
    });

    await test('update of existing mission succeeds (not quota-gated)', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitA', missionId: 'm1', accountId: 'acct2', name: 'M1-updated', data: { x: 1 } } });
        assert.strictEqual(last().ms.s, 'OK:save');
        assert.strictEqual(db.countMissionsByAccount('acct2'), 2);
    });

    console.log('\n--- Back-compat: no limits block => unbounded ---');

    await test('removing limits block makes news saves unbounded', async () => {
        cfg.limits = undefined;
        sent.length = 0;
        for (let i = 0; i < 5; i++) {
            await handlers.handleSaveNews('c1', { mt: 9016, ms: { newsId: `nb${i}`, scope: 'account', accountId: 'acct3', body: 'b' } });
            assert.strictEqual(last().ms.s, 'OK:save', `insert ${i} should succeed`);
        }
        assert.strictEqual(db.countNewsByAccount('acct3'), 5);
    });

    await test('removing limits block makes mission saves unbounded', async () => {
        sent.length = 0;
        for (let i = 0; i < 5; i++) {
            await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitD', missionId: `mb${i}`, accountId: 'acct4', name: 'M', data: {} } });
            assert.strictEqual(last().ms.s, 'OK:save', `insert ${i} should succeed`);
        }
        assert.strictEqual(db.countMissionsByAccount('acct4'), 5);
        assert.strictEqual(db.countMissionsByUnit('unitD', 'acct4'), 5);
    });

    // --- Cleanup ----------------------------------------------------------
    db.close();
    try { fs.rmSync(path.dirname(tmpDb), { recursive: true, force: true }); } catch (e) {}

    console.log(`\n=== Results: ${testsPassed} passed, ${testsFailed} failed ===\n`);
    process.exit(testsFailed > 0 ? 1 : 0);
})();

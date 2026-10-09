"use strict";

/**
 * Tests for the P1F-01 cloud mission store: kind/size/meta columns,
 * metadata-only listing, server-generated ids, cross-account id safety,
 * size limit, per-kind quotas, and the rq reply echo.
 *
 * Exercises the full stack: real DatabaseManager (temp DB), real
 * MessageHandlers, real config loader, mock wsServer that records envelopes.
 *
 * Run:  node tests/test_mission_kinds.js
 */

const assert = require('assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const Database = require('better-sqlite3');

const serverConfig = require('../js_serverConfig.js');
const DatabaseManager = require('../src/database');
const MessageHandlers = require('../src/messageHandlers');

serverConfig.init();
const cfg = serverConfig.m_configuration;

// Unbounded quotas for the main test run; per-test overrides where needed.
cfg.limits = {
    max_mission_bytes: 1024
};

const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'de-kinds-'));
const tmpDb = path.join(tmpDir, 't.db');
cfg.database.path = tmpDb;

// --- Mock wsServer --------------------------------------------------------
const sent = [];
const mockWsServer = {
    send: (connId, envelope) => sent.push(envelope),
    broadcast: () => {},
    getConnection: () => ({ commServerId: 'comm_test' })
};

let testsPassed = 0;
let testsFailed = 0;
function test(name, fn) {
    return Promise.resolve(fn()).then(
        () => { console.log(`  ✓ ${name}`); testsPassed++; },
        (e) => { console.error(`  ✗ ${name}\n    ${e.message}`); testsFailed++; }
    );
}

const last = () => sent[sent.length - 1];

// --- Migration test: build an OLD-schema DB first, then open it -----------
(async () => {
    console.log('\n=== Mission Kinds / Store Tests (P1F-01) ===\n');

    console.log('--- Migration on an old-schema DB ---');

    // Write a pre-P1F-01 row with the legacy schema (no kind/size/meta).
    const legacy = new Database(tmpDb);
    legacy.exec(`
        CREATE TABLE IF NOT EXISTS units (
            id TEXT PRIMARY KEY, name TEXT, comm_server_id TEXT,
            created_at INTEGER DEFAULT (strftime('%s','now')),
            updated_at INTEGER DEFAULT (strftime('%s','now'))
        );
        CREATE TABLE IF NOT EXISTS missions (
            id TEXT PRIMARY KEY, unit_id TEXT NOT NULL, account_id TEXT NOT NULL,
            name TEXT, data TEXT NOT NULL, version INTEGER DEFAULT 1,
            created_at INTEGER DEFAULT (strftime('%s','now')),
            updated_at INTEGER DEFAULT (strftime('%s','now')),
            FOREIGN KEY (unit_id) REFERENCES units(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS access_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT, unit_id TEXT, action TEXT NOT NULL,
            resource_type TEXT, resource_id TEXT, comm_server_id TEXT,
            created_at INTEGER DEFAULT (strftime('%s','now'))
        );
    `);
    legacy.prepare(`INSERT INTO units (id) VALUES ('unitX')`).run();
    legacy.prepare(`INSERT INTO missions (id, unit_id, account_id, name, data) VALUES ('old1','unitX','acctA','Old','{}')`).run();
    // Pre-P1F-01 saves inserted the client missionId verbatim — which was null
    // — and SQLite allows NULL in a TEXT PRIMARY KEY, so old rows share id NULL.
    legacy.prepare(`INSERT INTO missions (id, unit_id, account_id, name, data) VALUES (NULL,'unitX','acctA','NoId','{}')`).run();
    legacy.prepare(`INSERT INTO missions (id, unit_id, account_id, name, data) VALUES (NULL,'unitX','acctA','NoId2','{}')`).run();
    legacy.close();

    const db = new DatabaseManager();
    db.initialize();
    const handlers = new MessageHandlers(db, mockWsServer);

    await test('old rows gain kind=de_plan and a backfilled size', async () => {
        const m = db.getMission('old1', 'acctA');
        assert.strictEqual(m.kind, 'de_plan');
        assert.strictEqual(m.size, Buffer.byteLength('{}'));
        assert.strictEqual(m.meta, null);
    });

    await test('NULL-id rows get backfilled UUIDs and become loadable', async () => {
        const rows = db.db.prepare('SELECT id FROM missions WHERE name LIKE ?').all('NoId%');
        assert.strictEqual(rows.length, 2);
        for (const r of rows) {
            assert.ok(r.id && r.id.length === 36, `expected UUID, got ${r.id}`);
        }
        assert.notStrictEqual(rows[0].id, rows[1].id, 'backfilled ids must be unique');
        const listed = db.listMissionsMeta('acctA', {});
        assert.ok(listed.every(m => m.missionId != null), 'no listed row may have a null missionId');
        const m = db.getMission(rows[0].id, 'acctA');
        assert.strictEqual(m.name, 'NoId');
    });

    console.log('\n--- Save (9011) ---');

    await test('save without missionId gets a server UUID + version 1 reply', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitX', accountId: 'acctA', name: 'Auto', data: { a: 1 }, rq: 'r1' } });
        const ms = last().ms;
        assert.strictEqual(ms.s, 'OK:save');
        assert.ok(ms.missionId && ms.missionId.length === 36, `expected UUID, got ${ms.missionId}`);
        assert.strictEqual(ms.version, 1);
        assert.strictEqual(ms.rq, 'r1', 'rq must be echoed');
    });

    await test('save with kind + meta stores them', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: '_general_', missionId: 'op1', accountId: 'acctA', name: 'Op', kind: 'de_operation', meta: { op: 'op1', roles: ['solo'] }, data: { fileType: 'de_operation' } } });
        assert.strictEqual(last().ms.s, 'OK:save');
        const m = db.getMission('op1', 'acctA');
        assert.strictEqual(m.kind, 'de_operation');
        assert.deepStrictEqual(m.meta, { op: 'op1', roles: ['solo'] });
    });

    await test('de_opgraph kind saves, and a deterministic id upserts (no second row)', async () => {
        sent.length = 0;
        const c_meta = { op: 'op1', op_hash: 'sha256:abc', schema: 'de.opgraph/1' };
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: '_general_', missionId: 'sg-graph-abc', accountId: 'acctA', name: 'Op graph', kind: 'de_opgraph', meta: c_meta, data: { schema: 'de.opgraph/1', nodes: [], edges: [] } } });
        assert.strictEqual(last().ms.s, 'OK:save');
        const m = db.getMission('sg-graph-abc', 'acctA');
        assert.strictEqual(m.kind, 'de_opgraph');
        assert.strictEqual(m.meta.op_hash, 'sha256:abc');
        // re-writing under the same deterministic id updates the row in
        // place — a re-deployed op must not add a second graph row
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: '_general_', missionId: 'sg-graph-abc', accountId: 'acctA', name: 'Op graph', kind: 'de_opgraph', meta: c_meta, data: { v: 2 } } });
        assert.strictEqual(last().ms.version, 2);
        sent.length = 0;
        await handlers.handleLoadMission('c1', { mt: 9010, ms: { list: true, accountId: 'acctA', kind: 'de_opgraph' } });
        const c_rows = last().ms.missions.filter((r) => r.meta?.op_hash === 'sha256:abc');
        assert.strictEqual(c_rows.length, 1, 'same op_hash must list exactly one de_opgraph row');
    });

    await test('unknown kind is rejected', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitX', missionId: 'bad1', accountId: 'acctA', name: 'X', kind: 'waypoint_zip', data: {} } });
        assert.ok(last().ms.s.startsWith('ERROR:unknown mission kind'));
        assert.strictEqual(last().success, false);
    });

    await test('cross-account id reuse is rejected and the original row is untouched', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitX', missionId: 'op1', accountId: 'acctB', name: 'Hijack', data: { evil: true } } });
        assert.ok(last().ms.s.startsWith('ERROR:mission id belongs to another account'));
        const m = db.getMission('op1', 'acctA');
        assert.strictEqual(m.name, 'Op', 'original row must be unchanged');
        assert.strictEqual(m.version, 1, 'version must not increment');
    });

    await test('same-account id updates and bumps the version', async () => {
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitX', missionId: 'op1', accountId: 'acctA', name: 'Op v2', kind: 'de_operation', meta: { op: 'op1', roles: ['solo'] }, data: { fileType: 'de_operation', v: 2 } } });
        assert.strictEqual(last().ms.s, 'OK:save');
        assert.strictEqual(last().ms.version, 2);
        assert.strictEqual(db.getMission('op1', 'acctA').name, 'Op v2');
    });

    await test('oversize mission is rejected by max_mission_bytes', async () => {
        sent.length = 0;
        const big = { blob: 'x'.repeat(2048) };
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitX', missionId: 'big1', accountId: 'acctA', name: 'Big', data: big } });
        assert.ok(last().ms.s.startsWith('ERROR:mission too large'), `got: ${last().ms.s}`);
        assert.strictEqual(last().success, false);
    });

    await test('per-kind quota gates inserts of that kind only', async () => {
        cfg.limits.max_missions_per_kind = { de_operation: 1 };   // op1 already saved
        sent.length = 0;
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitX', missionId: 'op2', accountId: 'acctA', name: 'Op2', kind: 'de_operation', data: {} } });
        assert.ok(last().ms.s.includes('per kind de_operation'), `got: ${last().ms.s}`);
        // a de_plan insert is unaffected
        await handlers.handleSaveMission('c1', { mt: 9011, ms: { unitId: 'unitX', missionId: 'p1', accountId: 'acctA', name: 'P1', data: {} } });
        assert.strictEqual(last().ms.s, 'OK:save');
        delete cfg.limits.max_missions_per_kind;
    });

    console.log('\n--- Load (9010) ---');

    await test('list:true returns metadata rows without data, newest first', async () => {
        sent.length = 0;
        await handlers.handleLoadMission('c1', { mt: 9010, ms: { list: true, accountId: 'acctA', rq: 'r2' } });
        const ms = last().ms;
        assert.strictEqual(ms.s, 'OK');
        assert.strictEqual(ms.list, true);
        assert.strictEqual(ms.rq, 'r2');
        assert.ok(ms.missions.length >= 3);
        const op = ms.missions.find(m => m.missionId === 'op1');
        assert.ok(op, 'op1 must appear in the listing');
        assert.strictEqual(op.kind, 'de_operation');
        assert.strictEqual(op.version, 2);
        assert.ok(!('data' in op), 'metadata listing must not carry data');
        assert.deepStrictEqual(op.meta, { op: 'op1', roles: ['solo'] });
    });

    await test('list:true with kind filter returns only that kind', async () => {
        sent.length = 0;
        await handlers.handleLoadMission('c1', { mt: 9010, ms: { list: true, accountId: 'acctA', kind: 'de_operation' } });
        const ids = last().ms.missions.map(m => m.missionId);
        assert.deepStrictEqual(ids, ['op1']);
    });

    await test('list:true rows are account-scoped', async () => {
        sent.length = 0;
        await handlers.handleLoadMission('c1', { mt: 9010, ms: { list: true, accountId: 'acctB' } });
        assert.strictEqual(last().ms.missions.length, 0);
    });

    await test('missionId load returns the full row including data', async () => {
        sent.length = 0;
        await handlers.handleLoadMission('c1', { mt: 9010, ms: { unitId: 'unitX', missionId: 'op1', accountId: 'acctA' } });
        const m = last().ms.mission;
        assert.strictEqual(m.id, 'op1');
        assert.deepStrictEqual(m.data, { fileType: 'de_operation', v: 2 });
    });

    await test('legacy {unitId} load still returns all full rows', async () => {
        sent.length = 0;
        await handlers.handleLoadMission('c1', { mt: 9010, ms: { unitId: 'unitX', accountId: 'acctA' } });
        const ms = last().ms;
        assert.strictEqual(ms.s, 'OK');
        assert.ok(Array.isArray(ms.mission));
        assert.ok(ms.mission.every(m => m.data !== undefined), 'legacy form must include data');
    });

    // --- Cleanup ----------------------------------------------------------
    db.close();
    try { fs.rmSync(tmpDir, { recursive: true, force: true }); } catch (e) {}

    console.log(`\n=== Results: ${testsPassed} passed, ${testsFailed} failed ===\n`);
    process.exit(testsFailed > 0 ? 1 : 0);
})();

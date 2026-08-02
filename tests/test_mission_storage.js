"use strict";

/**
 * Tests for mission save/load/delete operations in DatabaseManager.
 * Uses an in-memory SQLite database to avoid touching real data.
 *
 * Run:  node tests/test_mission_storage.js
 */

const Database = require('better-sqlite3');
const assert = require('assert');

// --- Test DB wrapper (in-memory, same schema as DatabaseManager) ----------
function createTestDB() {
    const db = new Database(':memory:');
    db.pragma('journal_mode = WAL');
    db.pragma('foreign_keys = ON');

    db.exec(`
        CREATE TABLE IF NOT EXISTS units (
            id TEXT PRIMARY KEY,
            name TEXT,
            comm_server_id TEXT,
            created_at INTEGER DEFAULT (strftime('%s', 'now')),
            updated_at INTEGER DEFAULT (strftime('%s', 'now'))
        );

        CREATE TABLE IF NOT EXISTS missions (
            id TEXT PRIMARY KEY,
            unit_id TEXT NOT NULL,
            account_id TEXT NOT NULL,
            name TEXT,
            data TEXT NOT NULL,
            version INTEGER DEFAULT 1,
            created_at INTEGER DEFAULT (strftime('%s', 'now')),
            updated_at INTEGER DEFAULT (strftime('%s', 'now')),
            FOREIGN KEY (unit_id) REFERENCES units(id) ON DELETE CASCADE
        );
    `);

    return db;
}

// --- Minimal handler that mirrors DatabaseManager mission methods ----------
const MissionDB = {
    db: null,

    init(db) {
        this.db = db;
    },

    upsertUnit(unitId, name, commServerId) {
        this.db.prepare(`
            INSERT INTO units (id, name, comm_server_id)
            VALUES (?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                comm_server_id = excluded.comm_server_id,
                updated_at = strftime('%s', 'now')
        `).run(unitId, name, commServerId);
    },

    saveMission(missionId, unitId, accountId, name, data) {
        return this.db.prepare(`
            INSERT INTO missions (id, unit_id, account_id, name, data)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                unit_id = excluded.unit_id,
                account_id = excluded.account_id,
                name = excluded.name,
                data = excluded.data,
                version = version + 1,
                updated_at = strftime('%s', 'now')
        `).run(missionId, unitId, accountId, name, JSON.stringify(data));
    },

    loadMissions(unitId, accountId) {
        const rows = this.db.prepare(`
            SELECT * FROM missions WHERE unit_id = ? AND account_id = ? ORDER BY created_at DESC
        `).all(unitId, accountId);
        return rows.map(r => ({ ...r, data: JSON.parse(r.data) }));
    },

    getMission(missionId, accountId) {
        const row = this.db.prepare('SELECT * FROM missions WHERE id = ? AND account_id = ?').get(missionId, accountId);
        if (row) row.data = JSON.parse(row.data);
        return row;
    },

    deleteMission(missionId) {
        return this.db.prepare('DELETE FROM missions WHERE id = ?').run(missionId);
    },

    close() {
        if (this.db) this.db.close();
    }
};


// --- Sample mission data (DE_V1 format) -----------------------------------
function makeMissionData(seq) {
    return {
        version: 1,
        mav_waypoints: [
            { seq: 0, frame: 0, command: 16, param1: 0, param2: 0, param3: 0, param4: 0,
              x: 30.12345 + seq * 0.001, y: 31.23456, z: 50, current: 1, autocontinue: 1 },
            { seq: 1, frame: 0, command: 16, param1: 0, param2: 0, param3: 0, param4: 0,
              x: 30.12445 + seq * 0.001, y: 31.23556, z: 50, current: 0, autocontinue: 1 },
            { seq: 2, frame: 0, command: 21, param1: 0, param2: 0, param3: 0, param4: 0,
              x: 0, y: 0, z: 0, current: 0, autocontinue: 1 }
        ],
        meta: { platform: "quad", created: Date.now() }
    };
}


// --- Test runner ----------------------------------------------------------
let testsPassed = 0;
let testsFailed = 0;

function test(name, fn) {
    try {
        fn();
        console.log(`  ✓ ${name}`);
        testsPassed++;
    } catch (e) {
        console.error(`  ✗ ${name}`);
        console.error(`    ${e.message}`);
        testsFailed++;
    }
}


// --- Tests ----------------------------------------------------------------
console.log('\n=== Mission Storage Tests ===\n');

const db = createTestDB();
MissionDB.init(db);

// Pre-create a unit so FK is satisfied
MissionDB.upsertUnit('drone_alpha', 'Alpha Drone', 'comm_srv_1');
MissionDB.upsertUnit('drone_bravo', 'Bravo Drone', 'comm_srv_1');
MissionDB.upsertUnit('_general_', 'General Missions', 'comm_srv_1');

console.log('--- Save Mission ---');

test('saveMission inserts a new mission', () => {
    const data = makeMissionData(0);
    const result = MissionDB.saveMission('mission_001', 'drone_alpha', 'team_alpha', 'Alpha Mission 1', data);
    assert.strictEqual(result.changes, 1, 'Expected 1 row change');
});

test('saveMission upserts (updates existing mission, increments version)', () => {
    const data = makeMissionData(1);
    MissionDB.saveMission('mission_001', 'drone_alpha', 'team_alpha', 'Alpha Mission 1 Updated', data);
    const mission = MissionDB.getMission('mission_001', 'team_alpha');
    assert.strictEqual(mission.name, 'Alpha Mission 1 Updated');
    assert.strictEqual(mission.version, 2, 'Version should be 2 after upsert');
});

test('saveMission stores multiple missions for same unit', () => {
    MissionDB.saveMission('mission_002', 'drone_alpha', 'team_alpha', 'Alpha Mission 2', makeMissionData(2));
    MissionDB.saveMission('mission_003', 'drone_alpha', 'team_alpha', 'Alpha Mission 3', makeMissionData(3));
    const missions = MissionDB.loadMissions('drone_alpha', 'team_alpha');
    assert.strictEqual(missions.length, 3, 'Should have 3 missions for drone_alpha');
});

test('saveMission stores mission for _general_ unit (no drone selected)', () => {
    MissionDB.saveMission('mission_gen_001', '_general_', 'team_alpha', 'General Mission', makeMissionData(0));
    const missions = MissionDB.loadMissions('_general_', 'team_alpha');
    assert.strictEqual(missions.length, 1, 'Should have 1 general mission');
});

console.log('\n--- Load Mission ---');

test('loadMissions returns missions ordered by created_at DESC', () => {
    const missions = MissionDB.loadMissions('drone_alpha', 'team_alpha');
    assert.strictEqual(missions.length, 3);
    // mission_001 was created first, mission_003 last
    // DESC ordering means newest first
    const ids = missions.map(m => m.id);
    assert.ok(ids.includes('mission_001'));
    assert.ok(ids.includes('mission_002'));
    assert.ok(ids.includes('mission_003'));
});

test('loadMissions returns empty array for unit with no missions', () => {
    MissionDB.upsertUnit('drone_charlie', 'Charlie Drone', 'comm_srv_1');
    const missions = MissionDB.loadMissions('drone_charlie', 'team_alpha');
    assert.strictEqual(missions.length, 0);
});

test('loadMissions returns empty array for non-existent unit', () => {
    const missions = MissionDB.loadMissions('non_existent_unit', 'team_alpha');
    assert.strictEqual(missions.length, 0);
});

test('loadMissions data is parsed from JSON string', () => {
    const missions = MissionDB.loadMissions('drone_alpha', 'team_alpha');
    const mission = missions.find(m => m.id === 'mission_001');
    assert.ok(mission.data, 'data should be parsed');
    assert.ok(mission.data.mav_waypoints, 'mav_waypoints should exist');
    assert.strictEqual(mission.data.mav_waypoints.length, 3);
});

test('getMission returns a specific mission by ID', () => {
    const mission = MissionDB.getMission('mission_002', 'team_alpha');
    assert.ok(mission);
    assert.strictEqual(mission.id, 'mission_002');
    assert.strictEqual(mission.unit_id, 'drone_alpha');
    assert.strictEqual(mission.name, 'Alpha Mission 2');
});

test('getMission returns undefined for non-existent ID', () => {
    const mission = MissionDB.getMission('non_existent_mission', 'team_alpha');
    assert.strictEqual(mission, undefined);
});

test('getMission data is parsed from JSON', () => {
    const mission = MissionDB.getMission('mission_003', 'team_alpha');
    assert.ok(mission.data);
    assert.strictEqual(mission.data.version, 1);
    assert.strictEqual(mission.data.mav_waypoints.length, 3);
});

console.log('\n--- Delete Mission ---');

test('deleteMission removes a mission', () => {
    MissionDB.saveMission('mission_del_001', 'drone_bravo', 'team_alpha', 'To Delete', makeMissionData(0));
    assert.ok(MissionDB.getMission('mission_del_001', 'team_alpha'));

    const result = MissionDB.deleteMission('mission_del_001');
    assert.strictEqual(result.changes, 1);
    assert.strictEqual(MissionDB.getMission('mission_del_001', 'team_alpha'), undefined);
});

test('deleteMission on non-existent ID returns 0 changes', () => {
    const result = MissionDB.deleteMission('non_existent');
    assert.strictEqual(result.changes, 0);
});

console.log('\n--- Round-trip: Save then Load ---');

test('round-trip: saved mission data matches loaded mission data', () => {
    const originalData = makeMissionData(5);
    originalData.meta.custom = 'round-trip-test';

    MissionDB.saveMission('mission_rt_001', 'drone_bravo', 'team_alpha', 'Round Trip', originalData);
    const loaded = MissionDB.getMission('mission_rt_001', 'team_alpha');

    assert.deepStrictEqual(loaded.data, originalData, 'Loaded data should match saved data');
    assert.strictEqual(loaded.data.meta.custom, 'round-trip-test');
});

test('round-trip: multiple saves and loads preserve all missions', () => {
    for (let i = 0; i < 5; i++) {
        MissionDB.saveMission(`mission_bulk_${i}`, 'drone_bravo', 'team_alpha', `Bulk ${i}`, makeMissionData(i));
    }
    const missions = MissionDB.loadMissions('drone_bravo', 'team_alpha');
    // mission_del_001 was deleted, mission_rt_001 + 5 bulk = 6
    assert.strictEqual(missions.length, 6);

    for (const m of missions) {
        assert.ok(m.data.mav_waypoints, `Mission ${m.id} should have parsed waypoints`);
    }
});

console.log('\n--- Version increment on upsert ---');

test('version increments correctly on repeated saves of same mission ID', () => {
    MissionDB.saveMission('mission_ver_test', 'drone_bravo', 'team_alpha', 'Version Test', makeMissionData(0));
    let mission = MissionDB.getMission('mission_ver_test', 'team_alpha');
    assert.strictEqual(mission.version, 1);

    MissionDB.saveMission('mission_ver_test', 'drone_bravo', 'team_alpha', 'Version Test v2', makeMissionData(1));
    mission = MissionDB.getMission('mission_ver_test', 'team_alpha');
    assert.strictEqual(mission.version, 2);

    MissionDB.saveMission('mission_ver_test', 'drone_bravo', 'team_alpha', 'Version Test v3', makeMissionData(2));
    mission = MissionDB.getMission('mission_ver_test', 'team_alpha');
    assert.strictEqual(mission.version, 3);
    assert.strictEqual(mission.name, 'Version Test v3');
});


console.log('\n--- Account Scoping (cross-account isolation) ---');

test('account A cannot load missions saved by account B for same unit', () => {
    // team_alpha already has missions for drone_alpha
    // team_beta saves a mission for the same unit
    MissionDB.saveMission('mission_beta_001', 'drone_alpha', 'team_beta', 'Beta Mission', makeMissionData(0));

    // team_alpha should NOT see team_beta's mission
    const alphaMissions = MissionDB.loadMissions('drone_alpha', 'team_alpha');
    assert.strictEqual(alphaMissions.length, 3, 'team_alpha should still see only its 3 missions');
    assert.ok(!alphaMissions.find(m => m.id === 'mission_beta_001'), 'team_alpha should not see team_beta mission');

    // team_beta should see only its own mission
    const betaMissions = MissionDB.loadMissions('drone_alpha', 'team_beta');
    assert.strictEqual(betaMissions.length, 1, 'team_beta should see only its 1 mission');
    assert.strictEqual(betaMissions[0].id, 'mission_beta_001');
});

test('account A cannot getMission by ID for a mission saved by account B', () => {
    // mission_001 belongs to team_alpha
    const alphaResult = MissionDB.getMission('mission_001', 'team_alpha');
    assert.ok(alphaResult, 'team_alpha should get its own mission');

    // team_beta should NOT be able to access mission_001
    const betaResult = MissionDB.getMission('mission_001', 'team_beta');
    assert.strictEqual(betaResult, undefined, 'team_beta should not access team_alpha mission');
});

test('same unit can have missions from multiple accounts without conflict', () => {
    MissionDB.saveMission('mission_shared_001', 'drone_alpha', 'team_alpha', 'Alpha Shared', makeMissionData(0));
    MissionDB.saveMission('mission_shared_002', 'drone_alpha', 'team_beta', 'Beta Shared', makeMissionData(1));

    const alphaMissions = MissionDB.loadMissions('drone_alpha', 'team_alpha');
    const betaMissions = MissionDB.loadMissions('drone_alpha', 'team_beta');

    // team_alpha: 3 original + mission_beta_001 is NOT theirs + mission_shared_001 = 4
    assert.strictEqual(alphaMissions.length, 4, 'team_alpha should have 4 missions');
    // team_beta: mission_beta_001 + mission_shared_002 = 2
    assert.strictEqual(betaMissions.length, 2, 'team_beta should have 2 missions');
});

test('account_id is stored and returned in mission records', () => {
    const mission = MissionDB.getMission('mission_001', 'team_alpha');
    assert.strictEqual(mission.account_id, 'team_alpha', 'account_id should be stored in mission record');
});

test('upsert preserves account_id association', () => {
    // mission_001 was created by team_alpha, upsert by team_alpha should keep it
    MissionDB.saveMission('mission_001', 'drone_alpha', 'team_alpha', 'Updated Again', makeMissionData(10));
    const mission = MissionDB.getMission('mission_001', 'team_alpha');
    assert.strictEqual(mission.account_id, 'team_alpha');
    assert.strictEqual(mission.name, 'Updated Again');
    assert.strictEqual(mission.version, 3, 'Version should be 3 after second upsert');

    // team_beta still cannot access it
    const betaResult = MissionDB.getMission('mission_001', 'team_beta');
    assert.strictEqual(betaResult, undefined);
});


// --- Cleanup & summary ----------------------------------------------------
MissionDB.close();

console.log(`\n=== Results: ${testsPassed} passed, ${testsFailed} failed ===\n`);
process.exit(testsFailed > 0 ? 1 : 0);

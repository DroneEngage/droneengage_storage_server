//! Mission storage (test_mission_storage.js, test_mission_kinds.js): saves,
//! loads, listing, kinds, meta, server ids, account isolation, limits and
//! the migration of an old-schema database.

mod common;

use common::*;
use de_storage_server::db::{s, Db, U};
use de_storage_server::logger::Log;
use serde_json::{json, Value};

fn save(r: &mut Rig, p_ms: Value) -> Value {
    r.send(json!({ "mt": 9011, "ms": p_ms }))["ms"].clone()
}

fn load(r: &mut Rig, p_ms: Value) -> Value {
    r.send(json!({ "mt": 9010, "ms": p_ms }))["ms"].clone()
}

#[test]
fn save_insert_upsert_and_version() {
    let mut r = rig(json!({}));
    assert_eq!(
        save(&mut r, json!({"unitId": "u1", "missionId": "m1", "accountId": "a", "name": "M", "data": {"w": [1, 2]}}))
            ["s"],
        "OK:save"
    );
    let v =
        save(&mut r, json!({"unitId": "u1", "missionId": "m1", "accountId": "a", "name": "M2", "data": {"w": [3]}}));
    assert_eq!(v["version"], 2);
    let m = r.storage.db.get_mission(&s("m1"), &s("a")).unwrap().unwrap();
    assert_eq!(m["name"], "M2");
    assert_eq!(m["data"], json!({"w": [3]}), "data is parsed back from JSON");
    for i in 3..6 {
        assert_eq!(
            save(&mut r, json!({"unitId": "u1", "missionId": "m1", "accountId": "a", "data": {}}))["version"],
            i
        );
    }
    // the unit row is created on save
    assert!(r.storage.db.get_unit(&s("u1")).unwrap().is_some());
}

#[test]
fn loads_are_account_scoped() {
    let mut r = rig(json!({}));
    save(&mut r, json!({"unitId": "_general_", "missionId": "ma", "accountId": "A", "data": {"x": 1}}));
    save(&mut r, json!({"unitId": "_general_", "missionId": "mb", "accountId": "B", "data": {"x": 2}}));
    let v = load(&mut r, json!({"unitId": "_general_", "accountId": "A"}));
    assert_eq!(v["mission"].as_array().unwrap().len(), 1);
    assert_eq!(v["mission"][0]["id"], "ma");
    assert_eq!(v["mission"][0]["account_id"], "A");
    // by id from the wrong account: no mission in the reply
    let v = load(&mut r, json!({"unitId": "_general_", "missionId": "mb", "accountId": "A"}));
    assert!(v.get("mission").is_none());
    assert_eq!(load(&mut r, json!({"unitId": "nobody", "accountId": "A"}))["mission"], json!([]));
    // delete needs the owning account
    assert_eq!(r.send(json!({"mt": 9012, "ms": {"missionId": "mb", "accountId": "A"}}))["ms"]["deleted"], 0);
    assert_eq!(r.send(json!({"mt": 9012, "ms": {"missionId": "mb", "accountId": "B"}}))["ms"]["deleted"], 1);
}

#[test]
fn kinds_meta_ids_and_listing() {
    let mut r = rig(json!({"limits": {"max_mission_bytes": 1024}}));
    let v =
        save(&mut r, json!({"unitId": "unitX", "accountId": "acctA", "name": "Auto", "data": {"a": 1}, "rq": "r1"}));
    assert_eq!(v["s"], "OK:save");
    assert_eq!(v["missionId"].as_str().unwrap().len(), 36, "a server-generated UUID");
    assert_eq!(v["version"], 1);
    assert_eq!(v["rq"], "r1", "rq is echoed");

    let v_meta = json!({"op": "op1", "roles": ["solo"]});
    let ms = json!({"unitId": "_general_", "missionId": "op1", "accountId": "acctA", "name": "Op", "kind": "de_operation", "meta": v_meta, "data": {"fileType": "de_operation"}});
    assert_eq!(save(&mut r, ms)["s"], "OK:save");
    let m = r.storage.db.get_mission(&s("op1"), &s("acctA")).unwrap().unwrap();
    assert_eq!(m["kind"], "de_operation");
    assert_eq!(m["meta"], v_meta);

    // a deterministic de_opgraph id updates in place
    let g = json!({"op": "op1", "op_hash": "sha256:abc", "schema": "de.opgraph/1"});
    save(
        &mut r,
        json!({"unitId": "_general_", "missionId": "sg", "accountId": "acctA", "kind": "de_opgraph", "meta": g, "data": {"v": 1}}),
    );
    assert_eq!(
        save(
            &mut r,
            json!({"unitId": "_general_", "missionId": "sg", "accountId": "acctA", "kind": "de_opgraph", "meta": g, "data": {"v": 2}})
        )["version"],
        2
    );
    let v = load(&mut r, json!({"list": true, "accountId": "acctA", "kind": "de_opgraph"}));
    assert_eq!(v["missions"].as_array().unwrap().len(), 1);

    let v = save(
        &mut r,
        json!({"unitId": "unitX", "missionId": "bad1", "accountId": "acctA", "kind": "waypoint_zip", "data": {}}),
    );
    assert_eq!(v["s"], "ERROR:unknown mission kind 'waypoint_zip'");

    // another account can't take over an id
    let v = save(
        &mut r,
        json!({"unitId": "unitX", "missionId": "op1", "accountId": "acctB", "name": "Hijack", "data": {"evil": true}}),
    );
    assert_eq!(v["s"], "ERROR:mission id belongs to another account");
    let m = r.storage.db.get_mission(&s("op1"), &s("acctA")).unwrap().unwrap();
    assert_eq!((m["name"].clone(), m["version"].clone()), (json!("Op"), json!(1)));

    let v = save(
        &mut r,
        json!({"unitId": "unitX", "missionId": "big1", "accountId": "acctA", "data": {"blob": "x".repeat(2048)}}),
    );
    assert!(v["s"].as_str().unwrap().starts_with("ERROR:mission too large ("), "{}", v);

    r.set("limits", json!({"max_mission_bytes": 1024, "max_missions_per_kind": {"de_operation": 1}}));
    let v = save(
        &mut r,
        json!({"unitId": "unitX", "missionId": "op2", "accountId": "acctA", "kind": "de_operation", "data": {}}),
    );
    assert!(v["s"].as_str().unwrap().contains("per kind de_operation"), "{}", v);
    assert_eq!(
        save(&mut r, json!({"unitId": "unitX", "missionId": "p1", "accountId": "acctA", "data": {}}))["s"],
        "OK:save"
    );

    // the listing has metadata, no data
    let v = load(&mut r, json!({"list": true, "accountId": "acctA", "rq": "r2"}));
    assert_eq!((v["s"].clone(), v["list"].clone(), v["rq"].clone()), (json!("OK"), json!(true), json!("r2")));
    let op = v["missions"].as_array().unwrap().iter().find(|m| m["missionId"] == "op1").unwrap().clone();
    assert!(op.get("data").is_none());
    assert_eq!(op["meta"], v_meta);
    let v = load(&mut r, json!({"list": true, "accountId": "acctA", "kind": "de_operation"}));
    assert_eq!(
        v["missions"].as_array().unwrap().iter().map(|m| m["missionId"].clone()).collect::<Vec<_>>(),
        [json!("op1")]
    );
    assert_eq!(load(&mut r, json!({"list": true, "accountId": "acctB"}))["missions"], json!([]));
    let v = load(&mut r, json!({"unitId": "unitX", "missionId": "op1", "accountId": "acctA"}));
    assert_eq!(v["mission"]["data"], json!({"fileType": "de_operation"}));
}

#[test]
fn an_old_database_is_migrated() {
    let v_dir = tempfile::tempdir().unwrap();
    let v_path = v_dir.path().join("old.db");
    {
        let c = rusqlite::Connection::open(&v_path).unwrap();
        c.execute_batch(
            "CREATE TABLE units (id TEXT PRIMARY KEY, name TEXT, comm_server_id TEXT, created_at INTEGER DEFAULT (strftime('%s','now')), updated_at INTEGER DEFAULT (strftime('%s','now')));
             CREATE TABLE missions (id TEXT PRIMARY KEY, unit_id TEXT NOT NULL, account_id TEXT NOT NULL, name TEXT, data TEXT NOT NULL, version INTEGER DEFAULT 1, created_at INTEGER DEFAULT (strftime('%s','now')), updated_at INTEGER DEFAULT (strftime('%s','now')));
             INSERT INTO units (id) VALUES ('unitX');
             INSERT INTO missions (id, unit_id, account_id, name, data) VALUES ('old1','unitX','acctA','Old','{}'), (NULL,'unitX','acctA','NoId','{}'), (NULL,'unitX','acctA','NoId2','{}');",
        )
        .unwrap();
    }
    let v_db = Db::open(&v_path, Log::silent()).unwrap();
    let m = v_db.get_mission(&s("old1"), &s("acctA")).unwrap().unwrap();
    assert_eq!((m["kind"].clone(), m["size"].clone(), m["meta"].clone()), (json!("de_plan"), json!(2), Value::Null));
    let v_rows = v_db.all("SELECT id FROM missions WHERE name LIKE 'NoId%'", &[]).unwrap();
    assert_eq!(v_rows.len(), 2);
    assert!(v_rows.iter().all(|r| r["id"].as_str().map(|t| t.len()) == Some(36)));
    assert_ne!(v_rows[0]["id"], v_rows[1]["id"]);
    let v_list = v_db.list_missions_meta(&s("acctA"), &U, &U, &U).unwrap();
    assert!(v_list.iter().all(|m| !m["missionId"].is_null()));
}

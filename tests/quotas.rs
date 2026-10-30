//! Per-account quotas (test_quota_limits.js).

mod common;

use common::*;
use de_storage_server::db::s;
use serde_json::json;

#[test]
fn news_quota() {
    let mut r = rig(json!({"limits": {"max_news_per_account": 2}}));
    let news = |r: &mut Rig, id: &str, acc: &str| {
        r.send(json!({"mt": 9016, "ms": {"newsId": id, "scope": "account", "accountId": acc, "body": "b"}}))["ms"]["s"]
            .clone()
    };
    assert_eq!(news(&mut r, "n1", "acct1"), "OK:save");
    assert_eq!(news(&mut r, "n2", "acct1"), "OK:save");
    assert_eq!(news(&mut r, "n3", "acct1"), "ERROR:news quota exceeded (2/2)");
    assert_eq!(news(&mut r, "n1", "acct1"), "OK:save", "an update is not quota-gated");
    let v = r.send(json!({"mt": 9016, "ms": {"newsId": "g1", "scope": "global", "body": "g"}}));
    assert_eq!(v["ms"]["s"], "OK:save", "global news is exempt");
    assert_eq!(r.storage.db.count_news_by_account(&s("acct1")).unwrap(), 2.0);
    r.storage.db.disable_news(&s("n2")).unwrap();
    assert_eq!(news(&mut r, "n4", "acct1"), "OK:save", "disabled news frees quota");
}

#[test]
fn mission_quotas() {
    let mut r = rig(json!({"limits": {"max_missions_per_account": 2, "max_missions_per_unit": 1}}));
    let save = |r: &mut Rig, unit: &str, id: &str| {
        r.send(json!({"mt": 9011, "ms": {"unitId": unit, "missionId": id, "accountId": "acct2", "data": {}}}))["ms"]
            ["s"]
            .clone()
    };
    assert_eq!(save(&mut r, "unitA", "m1"), "OK:save");
    assert_eq!(save(&mut r, "unitA", "m2"), "ERROR:mission quota exceeded (1/1 per unit)");
    assert_eq!(save(&mut r, "unitB", "m3"), "OK:save");
    assert_eq!(save(&mut r, "unitC", "m4"), "ERROR:mission quota exceeded (2/2 per account)");
    assert_eq!(save(&mut r, "unitA", "m1"), "OK:save", "an update is not quota-gated");
    assert_eq!(r.storage.db.count_missions_by_account(&s("acct2")).unwrap(), 2.0);
}

#[test]
fn no_limits_block_is_unbounded() {
    let mut r = rig(json!({}));
    for i in 0..5 {
        let v = r.send(json!({"mt": 9016, "ms": {"newsId": format!("nb{}", i), "scope": "account", "accountId": "acct3", "body": "b"}}));
        assert_eq!(v["ms"]["s"], "OK:save");
        let v = r.send(json!({"mt": 9011, "ms": {"unitId": "unitD", "missionId": format!("mb{}", i), "accountId": "acct4", "data": {}}}));
        assert_eq!(v["ms"]["s"], "OK:save");
    }
    assert_eq!(r.storage.db.count_missions_by_unit(&s("unitD"), &s("acct4")).unwrap(), 5.0);
}

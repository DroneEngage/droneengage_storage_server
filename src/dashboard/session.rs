//! express-session with its MemoryStore, as the dashboard configures it:
//! cookie `connect.sid` = `s:<id>.<HMAC-SHA256(id, secret)>`, HttpOnly,
//! SameSite=Strict, Secure when enable_SSL, two-hour expiry renewed by each
//! request; a session is stored (and the cookie sent) only once it holds
//! something (saveUninitialized: false).

use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde_json::{json, Map, Value};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Mutex;
use subtle::ConstantTimeEq;

use crate::util::now_ms;

pub const COOKIE_NAME: &str = "connect.sid";
pub const MAX_AGE_MS: i64 = 2 * 60 * 60 * 1000;

/// cookie-signature sign: value.base64(HMAC) without padding.
pub fn sign(p_value: &str, p_secret: &str) -> String {
    let mut v_mac = Hmac::<Sha256>::new_from_slice(p_secret.as_bytes()).expect("any key length");
    v_mac.update(p_value.as_bytes());
    let v_sig = base64::engine::general_purpose::STANDARD.encode(v_mac.finalize().into_bytes());
    format!("{}.{}", p_value, v_sig.trim_end_matches('='))
}

/// cookie-signature unsign.
pub fn unsign(p_signed: &str, p_secret: &str) -> Option<String> {
    let v_value = &p_signed[..p_signed.rfind('.')?];
    let v_expected = sign(v_value, p_secret);
    bool::from(v_expected.as_bytes().ct_eq(p_signed.as_bytes())).then(|| v_value.to_string())
}

/// uid-safe(24): a URL-safe random id.
fn new_id() -> String {
    let mut b = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

struct Stored {
    data: Map<String, Value>,
    expires: i64,
}

/// The session of one request.
pub struct Session {
    pub id: String,
    pub data: Map<String, Value>,
    m_original_id: String,
    m_original: String,
    pub destroyed: bool,
}

impl Session {
    pub fn get(&self, p_key: &str) -> Option<&Value> {
        self.data.get(p_key)
    }

    pub fn set(&mut self, p_key: &str, p_value: Value) {
        self.data.insert(p_key.into(), p_value);
    }

    /// req.session.adminAuthenticated (truthy)
    pub fn authenticated(&self) -> bool {
        !self.destroyed && crate::util::js_truthy(self.data.get("adminAuthenticated"))
    }

    fn modified(&self) -> bool {
        self.id != self.m_original_id || crate::util::js_stringify(&Value::Object(self.data.clone())) != self.m_original
    }
}

#[derive(Default)]
pub struct Store {
    m_sessions: Mutex<HashMap<String, Stored>>,
}

impl Store {
    /// The request's session: the stored one its signed cookie names, or a
    /// new empty one.
    pub fn load(&self, p_cookie: Option<&str>, p_secret: &str) -> Session {
        let v_id = p_cookie.and_then(|c| c.strip_prefix("s:")).and_then(|c| unsign(c, p_secret));
        let mut v_sessions = self.m_sessions.lock().unwrap();
        if let Some(v_id) = v_id {
            let v_live = v_sessions.get(&v_id).map(|s| s.expires > now_ms());
            match v_live {
                Some(true) => {
                    let v_data = v_sessions[&v_id].data.clone();
                    let v_original = crate::util::js_stringify(&Value::Object(v_data.clone()));
                    return Session {
                        id: v_id.clone(),
                        data: v_data,
                        m_original_id: v_id,
                        m_original: v_original,
                        destroyed: false,
                    };
                }
                Some(false) => {
                    v_sessions.remove(&v_id);
                }
                None => {}
            }
            // an unknown or expired id: a fresh session (generate())
        }
        let v_id = new_id();
        Session { id: v_id.clone(), data: Map::new(), m_original_id: v_id, m_original: "{}".into(), destroyed: false }
    }

    /// req.session.regenerate: drops the stored session, starts a new one.
    pub fn regenerate(&self, p_session: &mut Session) {
        self.m_sessions.lock().unwrap().remove(&p_session.id);
        p_session.id = new_id();
        p_session.data = Map::new();
    }

    /// req.session.destroy
    pub fn destroy(&self, p_session: &mut Session) {
        self.m_sessions.lock().unwrap().remove(&p_session.id);
        p_session.destroyed = true;
    }

    /// At the end of the response: saves a modified session (or touches an
    /// unchanged stored one); the Set-Cookie value when one is due.
    pub fn commit(
        &self,
        p_session: &Session,
        p_secret: &str,
        p_secure_cookie: bool,
        p_secure_conn: bool,
    ) -> Option<String> {
        if p_session.destroyed {
            return None;
        }
        let v_expires = now_ms() + MAX_AGE_MS;
        let v_modified = p_session.modified();
        let mut v_sessions = self.m_sessions.lock().unwrap();
        if v_modified {
            v_sessions.insert(p_session.id.clone(), Stored { data: p_session.data.clone(), expires: v_expires });
        } else if let Some(s) = v_sessions.get_mut(&p_session.id) {
            s.expires = v_expires;
        }
        drop(v_sessions);
        // shouldSetCookie: new or known, the cookie goes out when the session changed
        if !v_modified {
            return None;
        }
        // a Secure cookie is never sent over plain HTTP
        if p_secure_cookie && !p_secure_conn {
            return None;
        }
        let v_value = format!("s:{}", sign(&p_session.id, p_secret));
        Some(format!(
            "{}={}; Path=/; Expires={}; HttpOnly{}; SameSite=Strict",
            COOKIE_NAME,
            super::http::encode_component(&v_value),
            super::http::http_date(v_expires),
            if p_secure_cookie { "; Secure" } else { "" }
        ))
    }

    /// The stored sessions (tests).
    pub fn len(&self) -> usize {
        self.m_sessions.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `{error: ...}` etc. as stored data.
pub fn value(p_text: &str) -> Value {
    json!(p_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_like_cookie_signature() {
        // cookie-signature's documented example
        assert_eq!(sign("hello", "tobiiscool"), "hello.DGDUkGlIkCzPz+C0B064FNgHdEjox7ch8tOBGslZ5QI");
        assert_eq!(unsign("hello.DGDUkGlIkCzPz+C0B064FNgHdEjox7ch8tOBGslZ5QI", "tobiiscool").as_deref(), Some("hello"));
        assert_eq!(unsign("hello.DGDUkGlIkCzPz+C0B064FNgHdEjox7ch8tOBGslZ5QI", "other"), None);
    }

    #[test]
    fn stored_only_once_modified() {
        let v_store = Store::default();
        let s = v_store.load(None, "k");
        assert_eq!(v_store.commit(&s, "k", true, true), None);
        let mut s = v_store.load(None, "k");
        s.set("error", value("x"));
        let v_cookie = v_store.commit(&s, "k", true, true).unwrap();
        assert!(v_cookie.starts_with("connect.sid=s%3A"));
        let v_raw =
            crate::dashboard::http::percent_decode(v_cookie.split(';').next().unwrap().split_once('=').unwrap().1)
                .unwrap();
        let s2 = v_store.load(Some(&v_raw), "k");
        assert_eq!(s2.id, s.id);
        assert_eq!(s2.get("error"), Some(&value("x")));
        assert_eq!(v_store.commit(&s2, "k", true, true), None, "unchanged: no cookie");
        assert_eq!(v_store.load(Some(&v_raw), "wrong").data.len(), 0);
        // a Secure cookie is not set over plain HTTP
        let mut s3 = v_store.load(None, "k");
        s3.set("a", json!(1));
        assert_eq!(v_store.commit(&s3, "k", true, false), None);
    }
}

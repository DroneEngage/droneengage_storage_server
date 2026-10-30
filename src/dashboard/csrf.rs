//! csurf with `cookie: true`: the secret lives in the `_csrf` cookie, and a
//! token is `<salt>-<base64url(sha1(salt + '-' + secret))>`.

use base64::Engine as _;
use rand::Rng;
use sha1::{Digest, Sha1};
use subtle::ConstantTimeEq;

pub const COOKIE_NAME: &str = "_csrf";

/// csrf.secretSync(): 18 random bytes, base64url.
pub fn new_secret() -> String {
    let mut b = [0u8; 18];
    rand::thread_rng().fill(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn tokenize(p_secret: &str, p_salt: &str) -> String {
    let v_hash = base64::engine::general_purpose::STANDARD.encode(Sha1::digest(format!("{}-{}", p_salt, p_secret)));
    format!("{}-{}", p_salt, v_hash.replace('+', "-").replace('/', "_").replace('=', ""))
}

/// csrf.create(secret): a fresh salted token.
pub fn create(p_secret: &str) -> String {
    const CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut r = rand::thread_rng();
    let v_salt: String = (0..8).map(|_| CHARS[r.gen_range(0..CHARS.len())] as char).collect();
    tokenize(p_secret, &v_salt)
}

/// csrf.verify(secret, token)
pub fn verify(p_secret: &str, p_token: Option<&str>) -> bool {
    let Some(v_token) = p_token else { return false };
    let Some(i) = v_token.find('-') else { return false };
    let v_expected = tokenize(p_secret, &v_token[..i]);
    v_expected.len() == v_token.len() && bool::from(v_expected.as_bytes().ct_eq(v_token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_verify_against_their_secret() {
        let s = new_secret();
        assert_eq!(s.len(), 24);
        let t = create(&s);
        assert!(verify(&s, Some(&t)));
        assert!(!verify(&new_secret(), Some(&t)));
        assert!(!verify(&s, Some("nodash")));
        assert!(!verify(&s, None));
        // csrf's tokenize (salt "abcdefgh", secret "secret"), as Node computes it
        assert_eq!(
            tokenize("secret", "abcdefgh"),
            format!("abcdefgh-{}", {
                let h = base64::engine::general_purpose::STANDARD.encode(Sha1::digest("abcdefgh-secret"));
                h.replace('+', "-").replace('/', "_").replace('=', "")
            })
        );
    }
}

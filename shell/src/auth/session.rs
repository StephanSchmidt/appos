//! Signed cookies: the session, and the short-lived one that carries state
//! from the start of a sign-in to its callback.
//!
//! A cookie is `<payload>.<mac>`, both base64url: the payload is JSON with
//! the value and its expiry, the MAC an HMAC-SHA256 over the cookie's name
//! and the payload, with a key only this shell knows (`<data>/session.key`).
//! Nothing is stored: a request reads no database to know who is asking.
//! A new key signs everyone out.

use std::path::Path;

use axum::http::{HeaderMap, HeaderValue, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, KeyInit, Mac};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// Who is signed in.
pub const SESSION: &str = "appos-session";
/// A sign-in under way: state, nonce, where to go after.
pub const SIGN_IN: &str = "appos-signin";
/// How long a sign-in may take, from start to callback.
pub const SIGN_IN_SECS: i64 = 10 * 60;

const KEY_FILE: &str = "session.key";
const KEY_LEN: usize = 32;

pub struct Cookies {
    key: [u8; KEY_LEN],
    /// Only sent over https: when the site's origin is https.
    secure: bool,
}

#[derive(Serialize, Deserialize)]
struct Sealed<T> {
    /// Unix seconds.
    exp: i64,
    v: T,
}

impl Cookies {
    pub fn new(key: [u8; KEY_LEN], secure: bool) -> Self {
        Cookies { key, secure }
    }

    /// The same cookies on a listener at `origin`: Secure when it is https.
    pub fn on(&self, origin: &str) -> Cookies {
        Cookies { key: self.key, secure: origin.starts_with("https://") }
    }

    /// The key in `<data>/session.key`; made on first use, readable only
    /// by the shell's user.
    pub fn load_key(data: &Path) -> anyhow::Result<[u8; KEY_LEN]> {
        let path = data.join(KEY_FILE);
        if let Ok(bytes) = std::fs::read(&path) {
            return bytes.try_into().map_err(|_| anyhow::anyhow!("{} is not a session key: delete it and a new one is made", path.display()));
        }
        let mut key = [0u8; KEY_LEN];
        getrandom::fill(&mut key).map_err(|e| anyhow::anyhow!("no randomness for the session key: {e}"))?;
        std::fs::create_dir_all(data)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::io::Write::write_all(&mut options.open(&path)?, &key)?;
        Ok(key)
    }

    fn mac(&self, name: &str, payload: &str) -> Hmac<Sha256> {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.key).expect("HMAC takes keys of any length");
        mac.update(name.as_bytes());
        mac.update(b"\0");
        mac.update(payload.as_bytes());
        mac
    }

    /// `value` as the content of cookie `name`, valid for `secs`.
    pub fn seal<T: Serialize>(&self, name: &str, value: &T, secs: i64) -> String {
        let json = serde_json::to_vec(&Sealed { exp: now() + secs, v: value }).expect("a cookie value is plain data");
        let payload = B64.encode(json);
        let mac = B64.encode(self.mac(name, &payload).finalize().into_bytes());
        format!("{payload}.{mac}")
    }

    /// The value of cookie `name`, if this shell sealed it and it hasn't
    /// expired.
    pub fn open<T: DeserializeOwned>(&self, name: &str, sealed: &str) -> Option<T> {
        let (payload, mac) = sealed.split_once('.')?;
        self.mac(name, payload).verify_slice(&B64.decode(mac).ok()?).ok()?;
        let sealed: Sealed<T> = serde_json::from_slice(&B64.decode(payload).ok()?).ok()?;
        (sealed.exp > now()).then_some(sealed.v)
    }

    /// Reads cookie `name` from the request and opens it.
    pub fn read<T: DeserializeOwned>(&self, headers: &HeaderMap, name: &str) -> Option<T> {
        self.open(name, &cookie(headers, name)?)
    }

    /// A Set-Cookie header for `value`, kept `secs` by the browser.
    pub fn set<T: Serialize>(&self, name: &str, value: &T, secs: i64) -> HeaderValue {
        self.header(name, &self.seal(name, value, secs), secs)
    }

    /// A Set-Cookie header that removes cookie `name`.
    pub fn clear(&self, name: &str) -> HeaderValue {
        self.header(name, "", 0)
    }

    fn header(&self, name: &str, value: &str, secs: i64) -> HeaderValue {
        // Lax: the identity provider's redirect back is a top-level GET,
        // which must carry the sign-in cookie.
        let secure = if self.secure { "; Secure" } else { "" };
        HeaderValue::try_from(format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={secs}{secure}")).expect("sealed cookies are base64url")
    }
}

/// The value of cookie `name` in the request's Cookie headers.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(name)?.strip_prefix('=').map(str::to_string))
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// `n` random bytes as base64url, for state, nonces and PKCE verifiers.
pub fn random(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    B64.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookies(key: u8) -> Cookies {
        Cookies::new([key; KEY_LEN], false)
    }

    #[test]
    fn a_sealed_value_opens_with_the_same_key_and_name_only() {
        let sealed = cookies(1).seal(SESSION, &"ann", 60);
        assert_eq!(cookies(1).open::<String>(SESSION, &sealed).as_deref(), Some("ann"));
        assert_eq!(cookies(2).open::<String>(SESSION, &sealed), None, "another shell's key");
        assert_eq!(cookies(1).open::<String>(SIGN_IN, &sealed), None, "a cookie can't pass for another one");
    }

    #[test]
    fn tampered_or_expired_cookies_are_nobody() {
        let c = cookies(1);
        let sealed = c.seal(SESSION, &"ann", 60);
        let (payload, mac) = sealed.split_once('.').unwrap();
        let forged = B64.encode(serde_json::to_vec(&Sealed { exp: now() + 60, v: "bob" }).unwrap());
        assert_eq!(c.open::<String>(SESSION, &format!("{forged}.{mac}")), None);
        assert_eq!(c.open::<String>(SESSION, payload), None);
        assert_eq!(c.open::<String>(SESSION, "not.base64!"), None);
        assert_eq!(c.open::<String>(SESSION, &c.seal(SESSION, &"ann", -1)), None);
    }

    #[test]
    fn cookies_are_read_from_any_cookie_header() {
        let mut headers = HeaderMap::new();
        headers.append(header::COOKIE, HeaderValue::from_static("a=1; appos-session-x=2"));
        headers.append(header::COOKIE, HeaderValue::from_static("appos-session=3"));
        assert_eq!(cookie(&headers, SESSION).as_deref(), Some("3"));
        assert_eq!(cookie(&headers, "b"), None);
    }

    #[test]
    fn the_key_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let key = Cookies::load_key(dir.path()).unwrap();
        assert_eq!(Cookies::load_key(dir.path()).unwrap(), key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(KEY_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(dir.path().join(KEY_FILE), b"short").unwrap();
        assert!(Cookies::load_key(dir.path()).is_err());
    }

    #[test]
    fn set_cookie_headers_are_http_only_and_secure_on_https() {
        let h = Cookies::new([1; KEY_LEN], true).set(SESSION, &"ann", 60);
        let h = h.to_str().unwrap();
        assert!(h.starts_with("appos-session=") && h.contains("HttpOnly") && h.contains("SameSite=Lax") && h.ends_with("; Secure"));
        assert!(!cookies(1).clear(SESSION).to_str().unwrap().contains("Secure"));
    }
}

//! Tokens for the platform API: what `appos login` keeps and the CLI sends
//! as `Authorization: Bearer`. A signed-in admin makes one on the admin
//! screen (/admin/token) and pastes it into the CLI.
//!
//! A token acts as the person who made it: every call needs the policy's
//! `admin` for them, as the policy is now, so taking someone out of the
//! policy stops their tokens at once. Their groups are those of the sign-in
//! that made it. Only a hash is kept; a token lasts 30 days, and `appos
//! logout` ends it sooner.

use axum::http::{HeaderMap, header};
use sha2::Digest;

use super::Identity;
use super::session::random;
use crate::platform::now_ms;
use crate::shell::Shell;

/// How long a token lasts.
pub const DAYS: i64 = 30;
/// So a token is recognised when it turns up somewhere it shouldn't.
const PREFIX: &str = "appos_";

/// A new token for `who`; only its hash is kept.
pub async fn make(shell: &Shell, who: &Identity) -> anyhow::Result<String> {
    let token = format!("{PREFIX}{}", random(32));
    let (hash, person) = (hash(&token), serde_json::to_string(who)?);
    let expires_at = now_ms() + DAYS * 24 * 3_600_000;
    shell.platform.run(move |db| db.token_made(&hash, &person, expires_at)).await?;
    Ok(token)
}

/// Whose token this is: none if it is unknown or expired.
pub async fn who(shell: &Shell, token: &str) -> anyhow::Result<Option<Identity>> {
    let hash = hash(token);
    let person = shell.platform.run(move |db| db.token(&hash, now_ms())).await?;
    Ok(person.map(|p| serde_json::from_str(&p)).transpose()?)
}

/// Ends a token; false if it wasn't known.
pub async fn drop(shell: &Shell, token: &str) -> anyhow::Result<bool> {
    let hash = hash(token);
    shell.platform.run(move |db| db.token_dropped(&hash)).await
}

/// The token a request carries.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

fn hash(token: &str) -> String {
    sha2::Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_token_is_its_person_until_dropped() {
        let shell = crate::testing::shell(Default::default());
        let ann = Identity::new("1", "ann@acme.com", "Ann", ["platform".into()]).unwrap();
        let token = make(&shell, &ann).await.unwrap();
        assert!(token.starts_with("appos_") && token.len() > 40);
        assert_eq!(who(&shell, &token).await.unwrap(), Some(ann));
        assert_eq!(who(&shell, "appos_guess").await.unwrap(), None);
        assert!(drop(&shell, &token).await.unwrap());
        assert_eq!(who(&shell, &token).await.unwrap(), None);
        assert!(!drop(&shell, &token).await.unwrap());
    }

    #[test]
    fn expired_tokens_are_nobody() {
        let db = crate::platform::PlatformDb::in_memory();
        db.token_made("h", "{}", 1_000).unwrap();
        assert_eq!(db.token("h", 999).unwrap().as_deref(), Some("{}"));
        assert_eq!(db.token("h", 1_000).unwrap(), None);
    }

    #[test]
    fn the_bearer_is_read_from_authorization() {
        let headers = |v: &str| HeaderMap::from_iter([(header::AUTHORIZATION, v.parse().unwrap())]);
        assert_eq!(bearer(&headers("Bearer appos_x")), Some("appos_x"));
        assert_eq!(bearer(&headers("bearer  appos_x ")), Some("appos_x"));
        assert_eq!(bearer(&headers("Basic YTpi")), None);
        assert_eq!(bearer(&headers("Bearer ")), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }
}

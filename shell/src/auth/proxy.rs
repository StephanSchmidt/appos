//! The header provider: a proxy in front (oauth2-proxy, an ingress, a
//! zero-trust gateway) signs people in and sends who they are in headers
//! with every request. The shell keeps no session of its own. It believes
//! the headers only from the proxies auth.toml names, by the address the
//! request comes from, and ignores them from anyone else.
//!
//! ```toml
//! provider = "header"
//! trusted-proxies = ["10.0.0.2"]
//! email-header = "X-Forwarded-Email"     # these three are the defaults
//! name-header = "X-Forwarded-User"
//! groups-header = "X-Forwarded-Groups"   # comma separated
//! sign-in-url = "/oauth2/start?rd={to}"  # {to}: the page to come back to
//! sign-out-url = "/oauth2/sign_out"
//! ```

use std::net::IpAddr;

use axum::http::{HeaderMap, HeaderName};

use super::routes::Step;
use super::{Identity, Who, need, take_list, take_str};

pub struct Header {
    trusted: Vec<IpAddr>,
    email: HeaderName,
    name: HeaderName,
    groups: HeaderName,
    sign_in: Option<String>,
    sign_out: Option<String>,
}

impl Header {
    pub fn parse(table: &mut toml::Table) -> anyhow::Result<Header> {
        let trusted = need(take_list(table, "trusted-proxies")?, "header", "trusted-proxies = [\"10.0.0.2\"], the proxy's address")?;
        let trusted = trusted
            .iter()
            .map(|ip| ip.parse::<IpAddr>().map_err(|_| anyhow::anyhow!("trusted-proxies: {ip:?} is no IP address")))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let header = |table: &mut toml::Table, key: &str, default: &str| -> anyhow::Result<HeaderName> {
            let name = take_str(table, key)?.unwrap_or_else(|| default.to_string());
            HeaderName::try_from(name.to_ascii_lowercase()).map_err(|_| anyhow::anyhow!("{key} = {name:?} is no header name"))
        };
        let url = |table: &mut toml::Table, key: &str| -> anyhow::Result<Option<String>> {
            match take_str(table, key)? {
                Some(u) if crate::runtime::is_local(&u) || u.starts_with("https://") || u.starts_with("http://") => Ok(Some(u)),
                Some(u) => anyhow::bail!("{key} = {u:?}: give a path on this site or an http(s) URL"),
                None => Ok(None),
            }
        };
        Ok(Header {
            trusted,
            email: header(table, "email-header", "x-forwarded-email")?,
            name: header(table, "name-header", "x-forwarded-user")?,
            groups: header(table, "groups-header", "x-forwarded-groups")?,
            sign_in: url(table, "sign-in-url")?,
            sign_out: url(table, "sign-out-url")?,
        })
    }

    /// Who the proxy says sends the request; nobody if it didn't come
    /// through a trusted proxy.
    pub fn who(&self, headers: &HeaderMap, peer: Option<IpAddr>) -> Who {
        if !peer.is_some_and(|p| self.trusted.contains(&p)) {
            return Who::default();
        }
        let text = |name: &HeaderName| headers.get(name).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
        let Some(email) = text(&self.email) else { return Who::default() };
        let groups = text(&self.groups).unwrap_or_default();
        let name = text(&self.name).unwrap_or_default();
        // The user header of some proxies is the email again, or an id.
        let name = if name.contains('@') { "" } else { name.as_str() };
        match Identity::new("", &email, name, groups.split(',').map(str::to_string)) {
            Ok(id) => Who::signed_in(id),
            Err(_) => Who::default(),
        }
    }

    /// Signing in is the proxy's.
    pub fn start(&self, to: &str) -> Step {
        match &self.sign_in {
            Some(url) => Step::Redirect(url.replace("{to}", &form_urlencoded::byte_serialize(to.as_bytes()).collect::<String>())),
            None => Step::Refuse("This site signs you in before you reach it. Open the page again.".into()),
        }
    }

    pub fn sign_out(&self) -> String {
        self.sign_out.clone().unwrap_or_else(|| "/".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(text: &str) -> Header {
        Header::parse(&mut toml::from_str(text).unwrap()).unwrap()
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        pairs.iter().map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap())).collect()
    }

    #[test]
    fn headers_count_only_from_a_trusted_proxy() {
        let h = header("trusted-proxies = [\"10.0.0.2\", \"::1\"]");
        let sent = headers(&[("x-forwarded-email", "Ann@Acme.com"), ("x-forwarded-user", "Ann Lee"), ("x-forwarded-groups", " sales, platform ,")]);
        let who = h.who(&sent, Some("10.0.0.2".parse().unwrap()));
        let id = who.identity().unwrap();
        assert_eq!((id.email.as_str(), id.name.as_str()), ("ann@acme.com", "Ann Lee"));
        assert_eq!(id.groups, ["sales", "platform"]);
        assert!(h.who(&sent, Some("::1".parse().unwrap())).identity().is_some());
        assert!(h.who(&sent, Some("10.0.0.3".parse().unwrap())).identity().is_none(), "not from the proxy");
        assert!(h.who(&sent, None).identity().is_none());
        assert!(h.who(&headers(&[("x-forwarded-user", "ann")]), Some("10.0.0.2".parse().unwrap())).identity().is_none(), "no email");
    }

    #[test]
    fn header_names_and_urls_are_set_up_in_auth_toml() {
        let h = header(
            "trusted-proxies = [\"10.0.0.2\"]\nemail-header = \"X-Auth-Email\"\nsign-in-url = \"/oauth2/start?rd={to}\"\nsign-out-url = \"/oauth2/sign_out\"",
        );
        let who = h.who(&headers(&[("x-auth-email", "a@b.c"), ("x-forwarded-user", "a@b.c")]), Some("10.0.0.2".parse().unwrap()));
        assert_eq!(who.identity().unwrap().name, "a", "an email as the name is no name");
        let Step::Redirect(to) = h.start("/webinars/team?x=1") else { panic!() };
        assert_eq!(to, "/oauth2/start?rd=%2Fwebinars%2Fteam%3Fx%3D1");
        assert_eq!(h.sign_out(), "/oauth2/sign_out");
        assert!(matches!(header("trusted-proxies = [\"10.0.0.2\"]").start("/"), Step::Refuse(_)));

        let err = |text: &str| Header::parse(&mut toml::from_str(text).unwrap()).err().unwrap().to_string();
        assert!(err("").contains("needs trusted-proxies"));
        assert!(err("trusted-proxies = [\"proxy\"]").contains("no IP address"));
        assert!(err("trusted-proxies = [\"10.0.0.2\"]\nemail-header = \"a b\"").contains("no header name"));
        assert!(err("trusted-proxies = [\"10.0.0.2\"]\nsign-in-url = \"javascript:x\"").contains("give a path"));
    }
}

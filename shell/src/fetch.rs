//! Outgoing HTTP for kits: `appos:framework/http` for Wasm kits
//! (runtime.rs links it), `appos:auth/http` for sign-in kits
//! (auth/sign_in_kit.rs), the same shape. A kit fetches only from the
//! hosts in its kit.toml's [http], over https (http only to localhost),
//! without following redirects.

use std::time::Duration;

wasmtime::component::bindgen!({
    inline: "package appos:shell;
        world fetch { import appos:framework/http@0.1.0; }",
    path: "../wit",
    imports: { default: async | trappable },
});

pub(crate) use appos::framework::http;

/// Bytes of an answer.
const MAX_BODY: u64 = 1 << 20;

/// kit.toml's [http] hosts, checked: each a host alone, in lower case.
pub(crate) fn hosts(hosts: &[String]) -> anyhow::Result<Vec<String>> {
    hosts
        .iter()
        .map(|h| {
            let bad = h.is_empty() || h.contains(['/', ':', '@', '?', '#']) || h.chars().any(char::is_whitespace);
            anyhow::ensure!(!bad, "[http] hosts: {h:?}: give the host alone, like \"api.acme.com\"");
            Ok(h.to_lowercase())
        })
        .collect()
}

/// Whether a kit may fetch `url`: https to one of `hosts`, or http to one
/// of them on this machine.
pub(crate) fn allowed(url: &str, hosts: &[String]) -> Result<(), String> {
    let uri: ureq::http::Uri = url.parse().map_err(|_| format!("{url:?} is no URL"))?;
    let host = uri.host().unwrap_or_default().to_lowercase();
    let listed = hosts.contains(&host);
    let local = ["localhost", "127.0.0.1", "[::1]", "::1"].contains(&host.as_str());
    match uri.scheme_str() {
        _ if !listed => Err(format!("{host:?} is not in kit.toml's [http] hosts ({})", if hosts.is_empty() { "none".to_string() } else { hosts.join(", ") })),
        Some("https") => Ok(()),
        Some("http") if local => Ok(()),
        _ => Err(format!("{url:?}: only https (http only to localhost)")),
    }
}

/// Sends `req` off the async workers, within `left`. Check it is
/// `allowed` first.
pub(crate) async fn send(req: http::Request, left: Duration) -> wasmtime::Result<Result<http::Response, String>> {
    tokio::task::spawn_blocking(move || blocking(req, left)).await.map_err(|e| wasmtime::format_err!("fetch worker failed: {e}"))
}

fn blocking(req: http::Request, left: Duration) -> Result<http::Response, String> {
    if left.is_zero() {
        return Err("no time left".into());
    }
    let method = req.method.to_ascii_uppercase();
    if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
        return Err(format!("method {method}: give GET, POST, PUT, PATCH or DELETE"));
    }
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(left)).http_status_as_error(false).max_redirects(0).max_redirects_will_error(false).build().into();
    let mut builder = ureq::http::Request::builder().method(method.as_str()).uri(req.url.as_str());
    for (name, value) in &req.headers {
        if name.eq_ignore_ascii_case("host") {
            return Err("the host header is the URL's".into());
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    let answer = if req.body.is_empty() {
        agent.run(builder.body(()).map_err(|e| e.to_string())?)
    } else {
        agent.run(builder.body(req.body).map_err(|e| e.to_string())?)
    };
    let mut resp = answer.map_err(|e| e.to_string())?;
    let headers = resp.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string()))).collect();
    let status = resp.status().as_u16();
    let body = resp.body_mut().with_config().limit(MAX_BODY).read_to_string().map_err(|e| e.to_string())?;
    Ok(http::Response { status, headers, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kit_fetches_from_its_hosts_only() {
        let hosts = vec!["api.acme.com".to_string(), "localhost".to_string()];
        assert!(allowed("https://api.acme.com/leads", &hosts).is_ok());
        assert!(allowed("https://API.acme.com/leads", &hosts).is_ok());
        assert!(allowed("http://localhost:9000/x", &hosts).is_ok());
        assert!(allowed("http://api.acme.com/leads", &hosts).unwrap_err().contains("only https"));
        assert!(allowed("https://evil.com/", &hosts).unwrap_err().contains("not in kit.toml's [http] hosts (api.acme.com, localhost)"));
        assert!(allowed("https://api.acme.com.evil.com/", &hosts).is_err());
        assert!(allowed("https://api.acme.com@evil.com/", &hosts).is_err());
        assert!(allowed("/relative", &hosts).is_err());
        assert!(allowed("https://evil.com/", &[]).unwrap_err().contains("(none)"));
        assert!(allowed("ftp://api.acme.com/", &hosts).is_err());
    }

    #[test]
    fn hosts_are_hosts_alone() {
        assert_eq!(hosts(&["API.acme.com".into()]).unwrap(), ["api.acme.com"]);
        for bad in ["https://api.acme.com", "api.acme.com/x", "api.acme.com:443", "", "a b"] {
            assert!(hosts(&[bad.into()]).unwrap_err().to_string().contains("give the host alone"), "{bad}");
        }
    }
}

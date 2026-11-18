//! The token page: a token for the appos CLI (`appos login`), made by the
//! admin signed in here, as them (auth/tokens.rs). It is shown once.

use crate::auth::tokens;
use crate::kit::Reply;
use crate::platform::now_ms;
use crate::web::escape;

use super::model::{load, state};
use super::{Ask, page, read};

const HEAD: &str = r#"<div class="cardhead"><h2 id="token-h">Token for the appos CLI</h2></div>"#;

/// `GET /admin/token`.
pub(super) async fn token_page(ask: Ask) -> Reply {
    let url = ask.platform_url();
    let Ask { shell, viewer, .. } = ask;
    read(shell, move |shell| {
        let body = match viewer.who.identity() {
            Some(id) => format!(
                r#"<p>A token lets the appos CLI on your computer use this platform as <b>{email}</b>, with what policy.csv gives you: admin-read to read the log and the apps, admin-write to deploy, roll back and turn apps off and on. It lasts {days} days; <code>appos logout</code> ends it sooner.</p>
<form method="post" action="/admin/token"><button class="primary">Make a token</button></form>
<p class="foot">Then paste it into <code>appos login {url}</code>.</p>"#,
                email = escape(&id.email),
                days = tokens::DAYS,
                url = escape(&url),
            ),
            None => no_token(),
        };
        let (apps, _) = load(shell, now_ms());
        Reply::document(page(shell, &viewer, state(&apps), "", &format!("Token · {} admin", shell.name), &card(&body), None))
    })
    .await
}

/// `POST /admin/token`: makes one and shows it, this once.
pub(super) async fn make(ask: Ask) -> Reply {
    let url = ask.platform_url();
    let Ask { shell, viewer, .. } = ask;
    let (status, body) = match viewer.who.identity() {
        Some(id) => match tokens::make(&shell, id).await {
            Ok(token) => (
                200,
                format!(
                    r#"<p>Paste it into <code>appos login {url}</code>. It is shown only now; make another if it gets lost.</p>
<pre class="cmd"><code>{token}</code></pre>
<p class="foot">It acts as <b>{email}</b> for {days} days, with what policy.csv gives you then.</p>"#,
                    url = escape(&url),
                    token = escape(&token),
                    email = escape(&id.email),
                    days = tokens::DAYS,
                ),
            ),
            Err(e) => (500, format!("<p>The token could not be made: {}</p>", escape(&format!("{e:#}")))),
        },
        None => (200, no_token()),
    };
    read(shell, move |shell| {
        let (apps, _) = load(shell, now_ms());
        Reply::document(page(shell, &viewer, state(&apps), "", &format!("Token · {} admin", shell.name), &card(&body), None)).status(status)
    })
    .await
}

fn card(body: &str) -> String {
    format!(r#"<section class="card logcard token" aria-labelledby="token-h">{HEAD}{body}</section>"#)
}

/// Nobody is signed in: the shell runs with `--no-auth`.
fn no_token() -> String {
    "<p>The shell runs with <code>--no-auth</code>: its platform API takes no token, so <code>appos login</code> needs none.</p>".into()
}

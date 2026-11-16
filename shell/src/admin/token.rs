//! The token page: a token for the appos CLI (`appos login`), made by the
//! admin signed in here, as them (auth/tokens.rs). It is shown once.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::auth::tokens;
use crate::platform::now_ms;
use crate::web::escape;

use super::model::{load, state};
use super::{PlatformUrl, S, Viewer, page, read};

const HEAD: &str = r#"<div class="cardhead"><h2 id="token-h">Token for the appos CLI</h2></div>"#;

/// `GET /admin/token`.
pub(super) async fn token_page(State(shell): S, viewer: Viewer, PlatformUrl(url): PlatformUrl) -> Response {
    read(shell, move |shell| {
        let body = match viewer.who.identity() {
            Some(id) => format!(
                r#"<p>A token lets the appos CLI on your computer use this platform as <b>{email}</b>: deploy, roll back, turn apps off and on, read the log, while the policy makes you an admin. It lasts {days} days; <code>appos logout</code> ends it sooner.</p>
<form method="post" action="/admin/token"><button class="primary">Make a token</button></form>
<p class="foot">Then paste it into <code>appos login {url}</code>.</p>"#,
                email = escape(&id.email),
                days = tokens::DAYS,
                url = escape(&url),
            ),
            None => no_token(),
        };
        let (apps, _) = load(shell, now_ms());
        Html(page(shell, &viewer, state(&apps), "", &format!("Token · {} admin", shell.name), &card(&body), None)).into_response()
    })
    .await
}

/// `POST /admin/token`: makes one and shows it, this once.
pub(super) async fn make(State(shell): S, viewer: Viewer, PlatformUrl(url): PlatformUrl) -> Response {
    let (status, body) = match viewer.who.identity() {
        Some(id) => match tokens::make(&shell, id).await {
            Ok(token) => (
                StatusCode::OK,
                format!(
                    r#"<p>Paste it into <code>appos login {url}</code>. It is shown only now; make another if it gets lost.</p>
<pre class="cmd"><code>{token}</code></pre>
<p class="foot">It acts as <b>{email}</b> for {days} days, while the policy makes you an admin.</p>"#,
                    url = escape(&url),
                    token = escape(&token),
                    email = escape(&id.email),
                    days = tokens::DAYS,
                ),
            ),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("<p>The token could not be made: {}</p>", escape(&format!("{e:#}")))),
        },
        None => (StatusCode::OK, no_token()),
    };
    read(shell, move |shell| {
        let (apps, _) = load(shell, now_ms());
        (status, Html(page(shell, &viewer, state(&apps), "", &format!("Token · {} admin", shell.name), &card(&body), None))).into_response()
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

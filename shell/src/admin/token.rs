//! The token page: a token for the appos CLI (`appos login`), made by the
//! admin signed in here, as them (auth/tokens.rs). It is shown once.

use crate::auth::tokens;
use crate::kit::Reply;
use crate::platform::now_ms;
use crate::shell::Shell;

use super::model::{load, state};
use super::{Ask, Viewer, html, page, read};

/// The token card (templates/admin/token.html).
#[derive(askama::Template)]
#[template(path = "admin/token.html")]
struct Card<'a> {
    shown: Shown,
    url: &'a str,
    days: i64,
}

/// What the card shows: the offer, a token made, why it wasn't, or that
/// the shell takes none (`--no-auth`).
enum Shown {
    Offer(String),
    Made(String, String),
    Failed(String),
    NoAuth,
}

/// `GET /admin/token`.
pub(super) async fn token_page(ask: Ask) -> Reply {
    let url = ask.platform_url();
    let Ask { shell, viewer, .. } = ask;
    let shown = viewer.who.identity().map_or(Shown::NoAuth, |id| Shown::Offer(id.email.clone()));
    read(shell, move |shell| draw(shell, &viewer, Card { shown, url: &url, days: tokens::DAYS }, 200)).await
}

/// `POST /admin/token`: makes one and shows it, this once.
pub(super) async fn make(ask: Ask) -> Reply {
    let url = ask.platform_url();
    let Ask { shell, viewer, .. } = ask;
    let (shown, status) = match viewer.who.identity() {
        Some(id) => match tokens::make(&shell, id).await {
            Ok(token) => (Shown::Made(id.email.clone(), token), 200),
            Err(e) => (Shown::Failed(format!("{e:#}")), 500),
        },
        None => (Shown::NoAuth, 200),
    };
    read(shell, move |shell| draw(shell, &viewer, Card { shown, url: &url, days: tokens::DAYS }, status)).await
}

fn draw(shell: &Shell, viewer: &Viewer, card: Card, status: u16) -> Reply {
    let (apps, _) = load(shell, now_ms());
    Reply::document(page(shell, viewer, state(&apps), "", &format!("Token · {} admin", shell.name), &html(&card), None)).status(status)
}

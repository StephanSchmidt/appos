//! The messages page: commands on their way from apps to kits (outbox.rs),
//! by kit, oldest first. A command that failed every try waits here for an
//! admin; Retry gives it its tries again, at once.

use std::collections::BTreeMap;

use crate::kit::{Reply, Sent};
use crate::outbox::MAX_ATTEMPTS;
use crate::platform::{Actor, Kind, NewEvent, now_ms};
use crate::shell::Shell;
use crate::terminal_logln as println;
use crate::tx::{self, Waiting};

use super::actions::back;
use super::format::{clock, plural};
use super::model::{App, load, name_of, state};
use super::{Ask, Viewer, html, page, read};

/// `GET /admin/messages`.
pub(super) async fn messages_page(ask: Ask) -> Reply {
    let Ask { shell, viewer, .. } = ask;
    read(shell, move |shell| messages_html(shell, &viewer)).await
}

fn messages_html(shell: &Shell, viewer: &Viewer) -> Reply {
    let now = now_ms();
    let (apps, _) = load(shell, now);
    let outbox = tx::outbox(&shell.db).map_err(|e| e.to_string());
    let list = outbox.as_ref().map(|waiting| groups(&apps, waiting, now)).map_err(Clone::clone);
    let body = html(&Messages { list, attempts: MAX_ATTEMPTS, changes: viewer.changes });
    Reply::document(page(shell, viewer, state(&apps), "/admin/messages", &format!("Messages · {} admin", shell.name), &body, None))
}

/// The page's card (templates/admin/messages.html): the messages by
/// interface, or why the outbox couldn't be read.
#[derive(askama::Template)]
#[template(path = "admin/messages.html")]
struct Messages<'a> {
    list: Result<Vec<Group<'a>>, String>,
    attempts: u32,
    /// Retry acts (admin-write).
    changes: bool,
}

/// The messages to one interface.
struct Group<'a> {
    interface: &'a str,
    count: String,
    failed: usize,
    rows: Vec<Row>,
}

/// One message: the function, from which app for whom, and its state.
struct Row {
    function: String,
    app_path: String,
    app_name: String,
    user: Option<String>,
    sent: String,
    error: Option<String>,
    state: State,
}

enum State {
    /// Failed every try: its id and the tries.
    Waits(String, u32),
    OnItsWay,
    /// Failed so often, tried again in so many seconds.
    Again(String, i64),
}

fn groups<'a>(apps: &[App], waiting: &'a [Waiting], now: i64) -> Vec<Group<'a>> {
    let mut by_interface: BTreeMap<&str, Vec<&Waiting>> = BTreeMap::new();
    for w in waiting {
        by_interface.entry(&w.interface).or_default().push(w);
    }
    let group = |(interface, ws): (&'a str, Vec<&Waiting>)| Group {
        interface,
        count: plural(ws.len(), "message"),
        failed: ws.iter().filter(|w| w.due.is_none()).count(),
        rows: ws.iter().map(|w| row(apps, w, now)).collect(),
    };
    by_interface.into_iter().map(group).collect()
}

fn row(apps: &[App], w: &Waiting, now: i64) -> Row {
    let sent: Sent = serde_json::from_str(&w.about).unwrap_or_default();
    let state = match w.due {
        None => State::Waits(w.id.clone(), w.attempts),
        Some(_) if w.attempts == 0 => State::OnItsWay,
        Some(due) => State::Again(plural(w.attempts as usize, "time"), ((due - now).max(0) + 999) / 1000),
    };
    Row {
        function: w.function.split_once('#').map_or(w.function.as_str(), |(_, f)| f).to_string(),
        app_path: form_urlencoded::byte_serialize(w.app.as_bytes()).collect(),
        app_name: name_of(apps, &w.app).to_string(),
        user: sent.user.map(|u| u.email),
        sent: clock(w.at),
        error: w.error.clone(),
        state,
    }
}

/// `POST /admin/messages/{id}/retry`: a failed command gets its tries
/// again, now.
pub(super) async fn retry(ask: Ask) -> Reply {
    let done = retry_now(&ask.shell, ask.id(), ask.viewer.who.person()).await;
    back(&ask.form(), "/admin/messages", done)
}

async fn retry_now(shell: &Shell, id: String, person: Option<String>) -> super::actions::Done {
    let db = shell.db.clone();
    let found = tokio::task::spawn_blocking(move || tx::retry(&db, &id, now_ms())).await.map_err(|e| e.to_string())?;
    let Some(w) = found.map_err(|e| format!("Could not retry: {e}"))? else {
        return Err("That message isn't waiting any more".into());
    };
    shell.outbox.wake();
    let message = format!("{} tried again", w.function);
    println!("{}: {message}", w.app);
    let said = message.clone();
    shell
        .platform
        .run(move |db| db.event(&NewEvent { person: person.as_deref(), ..NewEvent::new(&w.app, "", Kind::MessageRetried, Actor::Admin, &said) }))
        .await
        .map_err(|e| format!("Retried, but could not log it: {e:#}"))?;
    Ok((message, None))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    use super::*;

    async fn html(shell: &std::sync::Arc<Shell>) -> String {
        let request = axum::http::Request::builder().uri("/admin/messages").body(Body::empty()).unwrap();
        let response = crate::web::platform_router(shell.clone(), None).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        String::from_utf8(to_bytes(response.into_body(), 4 << 20).await.unwrap().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn a_failed_message_waits_on_the_page_until_retried() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        assert!(html(&shell).await.contains("No command waits"));

        let mut t = tx::Tx::default();
        let args = r#"["web"]"#.into();
        let about = r#"{"user":{"id":"ann","email":"ann@acme.com","name":"Ann","groups":[]},"may":[]}"#.into();
        t.send(tx::Outgoing { id: "m1".into(), interface: "acme:crm/leads".into(), function: "acme:crm/leads#add-lead".into(), args, about });
        assert!(t.commit(&shell.db, "test").unwrap());
        tx::failed(&shell.db, "m1", MAX_ATTEMPTS, None, "the CRM is down").unwrap();
        let page = html(&shell).await;
        for part in [
            "acme:crm/leads",
            "<code>add-lead</code>",
            "for ann@acme.com",
            "Failed 10 times, waits for you",
            "the CRM is down",
            r#"action="/admin/messages/m1/retry""#,
        ] {
            assert!(page.contains(part), "{part}");
        }

        let (message, undo) = retry_now(&shell, "m1".into(), None).await.unwrap();
        assert_eq!((message.as_str(), undo), ("acme:crm/leads#add-lead tried again", None));
        assert_eq!(shell.platform.events(Some("test"), 1).unwrap()[0].kind, Kind::MessageRetried);
        assert!(html(&shell).await.contains("On its way"));
        assert_eq!(retry_now(&shell, "m1".into(), None).await.unwrap_err(), "That message isn't waiting any more");
    }
}

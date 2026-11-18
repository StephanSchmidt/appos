//! The messages page: commands on their way from apps to kits (outbox.rs),
//! by kit, oldest first. A command that failed every try waits here for an
//! admin; Retry gives it its tries again, at once.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Response};

use crate::auth::Who;
use crate::kit::Sent;
use crate::outbox::MAX_ATTEMPTS;
use crate::platform::{Actor, Kind, NewEvent, now_ms};
use crate::shell::Shell;
use crate::terminal_logln as println;
use crate::tx::{self, Waiting};
use crate::web::escape;

use super::actions::back;
use super::format::{clock, plural};
use super::model::{App, load, name_of, state};
use super::{S, Viewer, page, read};

/// `GET /admin/messages`.
pub(super) async fn messages_page(State(shell): S, viewer: Viewer) -> Response {
    read(shell, move |shell| messages_html(shell, &viewer)).await
}

fn messages_html(shell: &Shell, viewer: &Viewer) -> Response {
    let now = now_ms();
    let (apps, _) = load(shell, now);
    let head = r#"<div class="cardhead"><h2 id="messages-h">Messages <small>· commands on their way from apps to kits</small></h2></div>"#;
    let list = match tx::outbox(&shell.db) {
        Ok(waiting) if waiting.is_empty() => r#"<p class="empty">No command waits: each one reached its kit.</p>"#.to_string(),
        Ok(waiting) => table(&apps, &waiting, now),
        Err(e) => format!(r#"<p class="empty">The outbox could not be read: {}</p>"#, escape(&e.to_string())),
    };
    let foot = format!(
        "An app's command waits here from the app's commit until its kit ran it. A failed one is tried again after 1 s, 2 s, 4 s and so on up to 5 minutes; after {MAX_ATTEMPTS} tries it waits for you."
    );
    let body = format!(r#"<section class="card logcard policy" aria-labelledby="messages-h">{head}{list}<p class="foot">{foot}</p></section>"#);
    Html(page(shell, viewer, state(&apps), "/admin/messages", &format!("Messages · {} admin", shell.name), &body, None)).into_response()
}

fn table(apps: &[App], waiting: &[Waiting], now: i64) -> String {
    let mut by_interface: BTreeMap<&str, Vec<&Waiting>> = BTreeMap::new();
    for w in waiting {
        by_interface.entry(&w.interface).or_default().push(w);
    }
    let rows: String = by_interface
        .iter()
        .flat_map(|(interface, ws)| {
            let failed = ws.iter().filter(|w| w.due.is_none()).count();
            let summary =
                format!(r#"<span class="id">{}{}</span>"#, plural(ws.len(), "message"), if failed > 0 { format!(", {failed} failed") } else { String::new() });
            let head = format!(r#"<tr><th scope="rowgroup" colspan="2"><span class="names"><span>{}</span>{summary}</span></th></tr>"#, escape(interface));
            std::iter::once(head).chain(ws.iter().map(move |w| row(apps, w, now)))
        })
        .collect();
    format!(r#"<div class="scroll"><table><thead><tr><th scope="col">Command</th><th scope="col">State</th></tr></thead><tbody>{rows}</tbody></table></div>"#)
}

fn row(apps: &[App], w: &Waiting, now: i64) -> String {
    let function = w.function.split_once('#').map_or(w.function.as_str(), |(_, f)| f);
    let app =
        format!(r#"<a href="/admin/apps/{}">{}</a>"#, form_urlencoded::byte_serialize(w.app.as_bytes()).collect::<String>(), escape(name_of(apps, &w.app)));
    let sent: Sent = serde_json::from_str(&w.about).unwrap_or_default();
    let user = sent.user.map(|u| format!(" for {}", escape(&u.email))).unwrap_or_default();
    let what =
        format!(r#"<span class="names"><span><code>{}</code></span><span class="id">from {app}{user}, sent {}</span></span>"#, escape(function), clock(w.at));
    let error = w.error.as_deref().map(|e| format!(r#"<span class="id">{}</span>"#, escape(e))).unwrap_or_default();
    let state = match w.due {
        None => format!(
            r#"<form method="post" action="/admin/messages/{}/retry"><span class="names"><span>Failed {} times, waits for you <button type="submit">Retry</button></span>{error}</span></form>"#,
            escape(&w.id),
            w.attempts
        ),
        Some(_) if w.attempts == 0 => r#"<span class="quiet">On its way</span>"#.to_string(),
        Some(due) => format!(
            r#"<span class="names"><span>Failed {}, next try in {} s</span>{error}</span>"#,
            plural(w.attempts as usize, "time"),
            ((due - now).max(0) + 999) / 1000
        ),
    };
    format!(r#"<tr><td>{what}</td><td class="may">{state}</td></tr>"#)
}

/// `POST /admin/messages/{id}/retry`: a failed command gets its tries
/// again, now.
pub(super) async fn retry(State(shell): S, who: Who, Path(id): Path<String>, axum::Form(form): axum::Form<BTreeMap<String, String>>) -> Response {
    let done = retry_now(&shell, id, who.person()).await;
    back(&form, "/admin/messages", done)
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

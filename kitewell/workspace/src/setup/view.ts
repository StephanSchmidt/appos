// The set-up page. The team, with the form to add someone, is a part of
// its own: adding someone with htmx brings back only that part, with the
// new invite link in it.

import type { Booking } from "../../bindings/flow.ts";
import { type Html, html } from "../appos.ts";
import { steps } from "../flow_view.ts";

export interface Page {
  base: string;
  booking: Booking;
  /** What was typed into the form, and why it was refused. */
  name: string;
  email: string;
  error: string;
}

export function page(t: Page): Html {
  const b = t.booking;
  return html`<div class="flow">
  ${steps(5)}

  <header class="intro ready">
    <div>
      <h1>Your workspace is ready</h1>
      <p class="lead">${b.company} on ${b.package}, for ${b.users} people. The invoices follow by email.</p>
    </div>
  </header>

  <div class="split">
    <section class="card" id="team">${team(t)}</section>

    <aside class="card summary">
      <h2>Your workspace</h2>
      <p><a class="workspace-link" href="${b.workspace}">${b.workspace}</a></p>
      ${b.project === undefined ? html`<p class="note">It starts empty.</p>` : html`<p class="note">Your first project, ${b.project}, is waiting there.</p>`}
      <a class="button" href="${b.workspace}">Open your workspace</a>
    </aside>
  </div>
</div>`;
}

/** The members, each with their invite link, and the form for the next. */
export function team(t: Page): Html {
  const b = t.booking;
  const seats = b.quote.billedUsers;
  const full = b.members.length >= seats;
  const action = `${t.base}/setup`;
  return html`<h2>Who else plans with you?</h2>
    <p class="note">${b.members.length} of ${seats} seats. Send each person their invite link. Kitewell sends no email from here.</p>
    <ul class="members">
      ${b.members.map(
        (m) => html`<li><span class="who">${m.name} <small>${m.email}</small></span><input type="text" readonly value="${m.invite}" aria-label="Invite link for ${m.name}"></li>`,
      )}
    </ul>
    ${
      full
        ? html`<p class="note">All ${seats} seats are taken.</p>`
        : html`<form class="add" method="post" action="${action}" hx-post="${action}" hx-swap="none">
      <input type="hidden" name="draft" value="${b.id}">
      <div class="add-fields">
        <div class="field"><label for="name">Name</label><input id="name" type="text" name="name" value="${t.name}" autocomplete="off"></div>
        <div class="field"><label for="email">Email</label><input id="email" type="email" name="email" value="${t.email}" autocomplete="off" aria-describedby="member_error"></div>
      </div>
      <p class="error" id="member_error" aria-live="polite">${t.error}</p>
      <div class="actions"><button class="quiet">Add and get their link</button></div>
    </form>`
    }`;
}

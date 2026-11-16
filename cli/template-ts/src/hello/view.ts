// The hello module's pages, as html templates. The page's parts that htmx
// swaps in (greeting, history, name_error) are functions of their own,
// used by the page and by the handler that updates them.

import { type Html, empty, html } from "../appos.ts";
import { type Greeting, NAME_MAX, type NameError, date, nameErrorText, time } from "./domain.ts";

/** What the page shows. */
export interface Index {
  base: string;
  /** Newest first; the first one is the greeting. */
  history: Greeting[];
  /** What was typed into the form, and why it didn't parse. */
  name: string;
  error: NameError | undefined;
}

export function index(t: Index): Html {
  return html`<div class="hello">
  <section class="intro">
    <span class="eyebrow">Your new app</span>
    <h1>Hello, <span class="gradient-text">world</span></h1>
    <p class="lead">This is your app. Make it yours: the page is src/hello/view.ts, the design src/style.ts, the logic src/hello/.</p>
  </section>

  <section class="card">
    <h2>Say hello</h2>
    <form class="greet" method="post" action="${t.base}/greet" hx-post="${t.base}/greet" hx-swap="none">
      <input type="text" name="name" value="${t.name}" placeholder="Your name" maxlength="${NAME_MAX}" autocomplete="off" aria-describedby="name_error">
      <button>Say hello</button>
    </form>
    <p class="error" id="name_error" aria-live="polite">${nameError(t)}</p>
  </section>

  <section class="greeting" id="greeting">${greeting(t)}</section>

  <section class="card">
    <h2>Recent hellos</h2>
    <ul class="history" id="history">${history(t)}</ul>
  </section>
</div>`;
}

export function nameError(t: Index): Html {
  return t.error === undefined ? empty : html`${nameErrorText(t.error)}`;
}

export function greeting(t: Index): Html {
  const g = t.history[0];
  if (g === undefined) return html`<p class="name">Hello!</p>`;
  return html`<p class="name">Hello, ${g.name}!</p>
      <p class="when">Said hello at ${time(g)}.</p>`;
}

export function history(t: Index): Html {
  if (t.history.length === 0) return html`<li class="empty">Nobody yet. Be the first!</li>`;
  return html`${t.history.map((g) => html`<li><a class="who" href="${t.base}/greetings/${g.id}">${g.name}</a><time>${time(g)}</time></li>`)}`;
}

/** One greeting's page. */
export function greetingPage(t: { base: string; greeting: Greeting }): Html {
  const g = t.greeting;
  return html`<div class="hello">
  <section class="greeting">
    <p class="name">Hello, ${g.name}!</p>
    <p class="when">Said hello on ${date(g)} at ${time(g)}.</p>
  </section>
  <p class="back"><a href="${t.base}">← All hellos</a></p>
</div>`;
}

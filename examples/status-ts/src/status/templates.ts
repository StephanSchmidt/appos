// The pages and their parts, as html templates: one function per page or
// part.

import { type Html, empty, html, raw } from "../appos.ts";
import { ICON } from "./assets.ts";
import type { Check, Chip, Errors, GroupView, IncidentView, MaintenanceView, MonthView, Opt, PartChoice } from "./view.ts";

// ---------------------------------------------------------------- public

/** Everything the status page shows. */
export interface Index {
  base: string;
  overall: string;
  overallKey: string;
  checked: string;
  checkedIso: string;
  open: IncidentView[];
  maintenance: MaintenanceView[];
  groups: GroupView[];
  months: MonthView[];
}

/** index.html */
export function index(t: Index): Html {
  return html`<div class="kw">
  <header class="masthead">
    <div class="wrap masthead-row">
      <a class="brand" href="${t.base}/"><span class="brand-icon" aria-hidden="true">${raw(ICON)}</span><span class="brand-name">Kitewell <span class="brand-sub">Status</span></span></a>
      <span class="masthead-note">All times in UTC</span>
    </div>
  </header>

  <div class="wrap stack content">
    <div id="live" class="stack" hx-get="${t.base}/live" hx-trigger="every 60s" hx-swap="none">${live(t)}</div>

    <section class="past" aria-labelledby="past-h">
      <h2 id="past-h">Past incidents</h2>
      ${t.months.map(
        (month) => html`
      <div class="month">
        <h3>${month.name}</h3>
        ${
          month.items.length === 0
            ? html`
        <p class="muted none">No incidents reported.</p>
        `
            : month.items.map(
                (p) => html`
        <article class="past-item">
          <h4>${p.maintenance ? html`<span class="tag">Maintenance</span> ${p.title}` : html`<a href="${t.base}/incidents/${p.id}">${p.title}</a>`}</h4>
          <p class="summary">${p.summary}</p>
          <p class="muted small">${p.when} · lasted ${p.lasted}</p>
          <p class="chips">${chips(p.chips)}</p>
        </article>
        `,
              )
        }
      </div>
      `,
      )}
    </section>
  </div>
</div>`;
}

/** The block of index.html that refreshes every minute. */
export function live(t: Index): Html {
  return html`
      <section class="overall s-${t.overallKey}" aria-live="polite">
        <span class="overall-mark" aria-hidden="true"></span>
        <div>
          <h1>${t.overall}</h1>
          <p class="muted">As of <time datetime="${t.checkedIso}">${t.checked}</time>. This page updates itself every minute.</p>
        </div>
      </section>

      ${t.open.map((i) => incident(t.base, i))}

      ${t.maintenance.map((m) => maintenance(m))}

      <section class="card systems" aria-labelledby="systems-h">
        <div class="section-head">
          <h2 id="systems-h">Systems</h2>
          <p class="muted small">Last 90 days. Point at or tap a day to see what happened.</p>
        </div>
        ${t.groups.map(
          (g) => html`
        <div class="group">
          ${
            g.single
              ? empty
              : html`
          <div class="group-head">
            <h3>${g.name}</h3>
            <span class="pill s-${g.statusKey}">${g.status}</span>
          </div>
          `
          }
          ${g.parts.map(
            (p) => html`
          <div class="part${g.single ? empty : html` in-group`}">
            <div class="part-head">
              ${g.single ? html`<h3 class="part-name">${p.name}</h3>` : html`<h4 class="part-name">${p.name}</h4>`}
              <span class="pill s-${p.statusKey}">${p.status}${p.note === "" ? empty : html` ${p.note}`}</span>
            </div>
            <div class="bars">${p.days.map((d) => html`<i class="s-${d.key}" tabindex="-1"><b>${d.tip}</b></i>`)}</div>
            <div class="scale"><span><span class="d90">90 days ago</span><span class="d30">30 days ago</span></span><span class="uptime">${p.uptime} uptime</span><span>Today</span></div>
          </div>
          `,
          )}
        </div>
        `,
        )}
        <ul class="legend" aria-label="What the colours mean">
          <li><span class="key s-operational"></span>Operational</li>
          <li><span class="key s-degraded"></span>Degraded performance</li>
          <li><span class="key s-partial"></span>Partial outage</li>
          <li><span class="key s-major"></span>Major outage</li>
          <li><span class="key s-maintenance"></span>Under maintenance</li>
        </ul>
        <p class="muted small">Uptime counts partial and major outages as down. Degraded performance and planned maintenance don't.</p>
      </section>
    `;
}

/** incident.html */
export function incidentPage(base: string, i: IncidentView): Html {
  return html`<div class="kw">
  <header class="masthead">
    <div class="wrap masthead-row">
      <a class="brand" href="${base}/"><span class="brand-icon" aria-hidden="true">${raw(ICON)}</span><span class="brand-name">Kitewell <span class="brand-sub">Status</span></span></a>
      <span class="masthead-note">All times in UTC</span>
    </div>
  </header>
  <div class="wrap stack content">
    <p class="back"><a href="${base}/">← All systems and incidents</a></p>
    ${incident(base, i)}
  </div>
</div>`;
}

function chips(cs: Chip[]): Html[] {
  return cs.map((c) => html`<span class="chip s-${c.key}">${c.name}<span class="chip-status"> · ${c.label}</span></span>`);
}

/** _incident.html */
function incident(base: string, i: IncidentView): Html {
  return html`<article class="card incident s-${i.stateKey}">
  <header class="card-head">
    <div>
      <p class="eyebrow">${i.open ? html`Open incident · ${i.when}` : html`Resolved incident · ${i.when} · lasted ${i.lasted}`}</p>
      <h2><a href="${base}/incidents/${i.id}">${i.title}</a></h2>
    </div>
    <span class="pill s-${i.stateKey}">${i.state}</span>
  </header>
  <p class="chips"><span class="muted small">Affects</span> ${chips(i.chips)}</p>
  ${i.nextUpdate === undefined ? empty : html`<p class="next">${i.nextUpdate}</p>`}
  <ol class="timeline">
    ${i.updates.map(
      (u) => html`
    <li class="stage-${u.stageKey}">
      <p class="stage">${u.stage}</p>
      <p class="message">${u.message}</p>
      <p class="when"><time datetime="${u.iso}">${u.stamp}</time> · ${u.ago}${u.edited ? html` · <span class="edited">edited</span>` : empty}</p>
    </li>
    `,
    )}
  </ol>
</article>`;
}

/** _maintenance.html */
function maintenance(m: MaintenanceView): Html {
  return html`<article class="card maintenance s-maintenance">
  <header class="card-head">
    <div>
      <p class="eyebrow">${m.phase}</p>
      <h2>${m.title}</h2>
    </div>
    <span class="pill s-maintenance">${m.phaseShort}</span>
  </header>
  <p class="window"><time>${m.when}</time> · ${m.relative}</p>
  <p class="chips"><span class="muted small">Affects</span> ${m.chips.map((c) => html`<span class="chip s-maintenance">${c.name}</span>`)}</p>
  <p class="message">${m.notice}</p>
  ${
    m.updates.length === 0
      ? empty
      : html`
  <ol class="timeline">
    ${m.updates.map(
      (u) => html`
    <li>
      <p class="stage">Update</p>
      <p class="message">${u.message}</p>
      ${u.newEnd === "" ? empty : html`<p class="new-end">${u.newEnd}</p>`}
      <p class="when"><time datetime="${u.iso}">${u.stamp}</time> · ${u.ago}</p>
    </li>
    `,
    )}
  </ol>
  `
  }
</article>`;
}

// ------------------------------------------------------------------ desk

/** What every desk page has at the top. */
export interface DeskTop {
  base: string;
  here: string;
  meQuery: string;
  nav: Opt[];
  saved: string;
}

/** _desk_top.html */
function deskTop(t: DeskTop): Html {
  return html`<header class="masthead">
  <div class="wrap masthead-row">
    <a class="brand" href="${t.base}/desk${t.meQuery}"><span class="brand-icon" aria-hidden="true">${raw(ICON)}</span><span class="brand-name">Kitewell <span class="brand-sub">Status desk</span></span></a>
    <a class="masthead-link" href="${t.base}/">Open the public page</a>
  </div>
</header>`;
}

/** _who.html */
function who(t: DeskTop): Html {
  return html`<nav class="who" aria-label="Who is on the desk">
  <span class="who-label">On the desk:</span>
  ${t.nav.map(
    (p) =>
      html`<a class="who-name${p.selected ? html` on` : empty}" href="${t.base}${t.here}?me=${p.value}"${p.selected ? html` aria-current="true"` : empty}>${p.label}</a>`,
  )}
</nav>
${t.saved === "" ? empty : html`<p class="saved" role="status">${t.saved}</p>`}`;
}

/** _rules.html */
const RULES = html`<aside class="rules" aria-label="Our writing rules">
  <h4>Our writing rules</h4>
  <ol>
    <li>Say what customers notice, not system names (“database”, “db-03”, “Kubernetes”).</li>
    <li>Never blame a vendor or another team.</li>
    <li>Say when the next update comes.</li>
  </ol>
</aside>`;

/** _message.html: the message box; the stage select replaces it with a
 * template. */
export function messageBox(id: string, text: string): Html {
  return html`<textarea id="${id}-box" name="message" rows="5" maxlength="2000" aria-describedby="${id}-message-error">${text}</textarea>`;
}

/** _parts.html: a status select per part; resolving resets it. */
export function partsBox(rows: PartChoice[]): Html {
  return html`<div class="choices">
  ${rows.map(
    (r) => html`
  <label class="choice">
    <span class="choice-name">${r.name}${r.note === "" ? empty : html` <small>${r.note}</small>`}</span>
    <select name="part-${r.key}">${options(r.options)}</select>
  </label>
  `,
  )}
</div>`;
}

function options(opts: Opt[]): Html[] {
  return opts.map((o) => html`<option value="${o.value}"${o.selected ? html` selected` : empty}>${o.label}</option>`);
}

export interface OpenForm {
  title: string;
  stages: Opt[];
  messageHtml: Html;
  partsHtml: Html;
  next: Opt[];
  by: Opt[];
  errors: Errors;
}

export interface PartsForm {
  partsHtml: Html;
  by: Opt[];
  errors: Errors;
}

export interface MaintForm {
  title: string;
  parts: Check[];
  starts: string;
  ends: string;
  notice: string;
  by: Opt[];
  earliest: string;
  earliestInput: string;
  errors: Errors;
}

export interface Desk extends DeskTop {
  open: IncidentView[];
  resolved: IncidentView[];
  maintenance: MaintenanceView[];
  completed: MaintenanceView[];
  openForm: OpenForm;
  partsForm: PartsForm;
  maintForm: MaintForm;
}

/** desk.html */
export function desk(t: Desk): Html {
  const f = t.openForm;
  const pf = t.partsForm;
  const mf = t.maintForm;
  return html`<div class="kw desk">
  ${deskTop(t)}
  <div class="wrap stack content">
    ${who(t)}

    <section class="card" aria-labelledby="open-h">
      <h2 id="open-h">Open incidents</h2>
      ${
        t.open.length === 0
          ? html`
      <p class="muted">No open incidents right now.</p>
      `
          : t.open.map(
              (i) => html`
      <a class="row-link" href="${t.base}/desk/incidents/${i.id}${t.meQuery}">
        <span class="row-main"><strong>${i.title}</strong><small>${i.when} · last update ${i.latestAgo}</small></span>
        <span class="pill s-${i.stateKey}">${i.state}</span>
      </a>
      `,
            )
      }
    </section>

    <section class="card" id="new-incident" aria-labelledby="new-h">
      <h2 id="new-h">Open an incident</h2>
      <form class="form" method="post" action="${t.base}/desk/incidents">
        <input type="hidden" name="form" value="open">
        <div class="field">
          <label for="open-title">Title, in customer words</label>
          <input id="open-title" name="title" value="${f.title}" maxlength="120" placeholder="Some users can't log in" autocomplete="off">
          <p class="error">${f.errors.get("title")}</p>
        </div>
        <fieldset class="field">
          <legend>Affected parts and how badly</legend>
          ${f.partsHtml}
          <p class="error">${f.errors.get("parts")}</p>
        </fieldset>
        <div class="compose">
          <div class="field">
            <label for="open-stage">Stage</label>
            <select id="open-stage" name="stage" hx-get="${t.base}/desk/compose" hx-trigger="change" hx-include="closest form" hx-swap="none">${options(f.stages)}</select>
            <p class="error">${f.errors.get("stage")}</p>
            <label for="open-box">First update for customers</label>
            <div id="open-message">${f.messageHtml}</div>
            <p class="hint">Picking a stage fills in its template. Replace the words in [brackets].</p>
            <p class="error" id="open-message-error">${f.errors.get("message")}</p>
          </div>
          ${RULES}
        </div>
        <div class="row">
          <div class="field">
            <label for="open-next">Next update</label>
            <select id="open-next" name="next">${options(f.next)}</select>
            <p class="error">${f.errors.get("next")}</p>
          </div>
          <div class="field">
            <label for="open-by">Posted by</label>
            <select id="open-by" name="by">${options(f.by)}</select>
            <p class="error">${f.errors.get("by")}</p>
          </div>
        </div>
        <div class="actions"><button class="button">Open incident</button></div>
      </form>
    </section>

    <section class="card" aria-labelledby="parts-h">
      <h2 id="parts-h">Parts right now</h2>
      <p class="muted">During an incident, change parts in its updates. Use this for the rest, like setting a part back to Operational that you kept at Degraded performance when you resolved an incident. Planned maintenance shows by itself.</p>
      <form class="form" method="post" action="${t.base}/desk/parts">
        ${pf.partsHtml}
        <p class="error">${pf.errors.get("parts")}</p>
        <div class="row">
          <div class="field">
            <label for="parts-by">Changed by</label>
            <select id="parts-by" name="by">${options(pf.by)}</select>
            <p class="error">${pf.errors.get("by")}</p>
          </div>
        </div>
        <div class="actions"><button class="button">Save part statuses</button></div>
      </form>
    </section>

    <section class="card" aria-labelledby="maint-h">
      <h2 id="maint-h">Planned maintenance</h2>
      ${
        t.maintenance.length === 0
          ? html`
      <p class="muted">Nothing planned.</p>
      `
          : t.maintenance.map(
              (m) => html`
      <a class="row-link" href="${t.base}/desk/maintenance/${m.id}${t.meQuery}">
        <span class="row-main"><strong>${m.title}</strong><small>${m.when} · ${m.relative}</small></span>
        <span class="pill s-maintenance">${m.phaseShort}</span>
      </a>
      `,
            )
      }

      <h3 id="schedule">Schedule maintenance</h3>
      <form class="form" method="post" action="${t.base}/desk/maintenance">
        <div class="field">
          <label for="maint-title">Title, in customer words</label>
          <input id="maint-title" name="title" value="${mf.title}" maxlength="120" placeholder="Kitewell will be read-only for up to an hour" autocomplete="off">
          <p class="error">${mf.errors.get("title")}</p>
        </div>
        <fieldset class="field">
          <legend>Parts it affects</legend>
          <div class="checks">${mf.parts.map((c) => html`<label class="check"><input type="checkbox" name="part-${c.key}"${c.checked ? html` checked` : empty}> ${c.name}</label>`)}</div>
          <p class="error">${mf.errors.get("parts")}</p>
        </fieldset>
        <div class="row">
          <div class="field">
            <label for="maint-starts">Starts (UTC)</label>
            <input id="maint-starts" type="datetime-local" name="starts" value="${mf.starts}" min="${mf.earliestInput}">
            <p class="error">${mf.errors.get("starts")}</p>
          </div>
          <div class="field">
            <label for="maint-ends">Ends (UTC)</label>
            <input id="maint-ends" type="datetime-local" name="ends" value="${mf.ends}" min="${mf.earliestInput}">
            <p class="error">${mf.errors.get("ends")}</p>
          </div>
        </div>
        <p class="hint">Times are UTC. We announce maintenance at least 3 days ahead: the earliest start is ${mf.earliest}.</p>
        <div class="compose">
          <div class="field">
            <label for="maint-notice">What customers will notice</label>
            <textarea id="maint-notice" name="notice" rows="4" maxlength="2000" placeholder="You can view everything but can't make changes during the window.">${mf.notice}</textarea>
            <p class="error">${mf.errors.get("notice")}</p>
          </div>
          ${RULES}
        </div>
        <div class="row">
          <div class="field">
            <label for="maint-by">Scheduled by</label>
            <select id="maint-by" name="by">${options(mf.by)}</select>
            <p class="error">${mf.errors.get("by")}</p>
          </div>
        </div>
        <div class="actions"><button class="button">Schedule maintenance</button></div>
      </form>
    </section>

    <section class="card" aria-labelledby="past-h">
      <h2 id="past-h">Past incidents and maintenance</h2>
      <p class="muted">Open one to see its whole history or fix a typo. Nothing is ever deleted.</p>
      ${
        t.resolved.length === 0
          ? html`
      <p class="muted">No past incidents.</p>
      `
          : t.resolved.map(
              (i) => html`
      <a class="row-link" href="${t.base}/desk/incidents/${i.id}${t.meQuery}">
        <span class="row-main"><strong>${i.title}</strong><small>${i.when} · lasted ${i.lasted}</small></span>
        <span class="pill s-operational">Resolved</span>
      </a>
      `,
            )
      }
      ${t.completed.map(
        (m) => html`
      <a class="row-link" href="${t.base}/desk/maintenance/${m.id}${t.meQuery}">
        <span class="row-main"><strong>${m.title}</strong><small>${m.when}</small></span>
        <span class="pill s-maintenance">Completed</span>
      </a>
      `,
      )}
    </section>
  </div>
</div>`;
}

export interface UpdateForm {
  stages: Opt[];
  messageHtml: Html;
  partsHtml: Html;
  next: Opt[];
  by: Opt[];
  errors: Errors;
}

export interface DeskIncident extends DeskTop {
  i: IncidentView;
  publicUrl: string;
  form: UpdateForm;
  editBy: Opt[];
}

/** desk_incident.html */
export function deskIncident(t: DeskIncident): Html {
  const i = t.i;
  const f = t.form;
  return html`<div class="kw desk">
  ${deskTop(t)}
  <div class="wrap stack content">
    ${who(t)}
    <p class="back"><a href="${t.base}/desk${t.meQuery}">← Back to the desk</a></p>

    <section class="card incident s-${i.stateKey}">
      <header class="card-head">
        <div>
          <p class="eyebrow">${i.open ? html`Open incident · ${i.when}` : html`Resolved incident · ${i.when} · lasted ${i.lasted}`}</p>
          <h1>${i.title}</h1>
        </div>
        <span class="pill s-${i.stateKey}">${i.state}</span>
      </header>
      <p class="chips"><span class="muted small">Affects</span> ${chips(i.chips)}</p>
      ${i.nextUpdate === undefined ? empty : html`<p class="next">${i.nextUpdate}</p>`}
      <p class="small">Customers see it at <a href="${t.publicUrl}">${t.publicUrl}</a>. Link it in your ticket macros.</p>
      ${i.open ? empty : html`<p class="error">${f.errors.get("stage")}</p>`}
    </section>

    ${
      !i.open
        ? empty
        : html`
    <section class="card" aria-labelledby="update-h">
      <h2 id="update-h">Post an update</h2>
      <form class="form" method="post" action="${t.base}/desk/incidents/${i.id}/updates">
        <input type="hidden" name="form" value="update">
        <input type="hidden" name="incident" value="${i.id}">
        <div class="compose">
          <div class="field">
            <label for="update-stage">Stage</label>
            <select id="update-stage" name="stage" hx-get="${t.base}/desk/compose" hx-trigger="change" hx-include="closest form" hx-swap="none">${options(f.stages)}</select>
            <p class="error">${f.errors.get("stage")}</p>
            <label for="update-box">Message for customers</label>
            <div id="update-message">${f.messageHtml}</div>
            <p class="hint">Picking a stage fills in its template. Replace the words in [brackets].</p>
            <p class="error" id="update-message-error">${f.errors.get("message")}</p>
          </div>
          ${RULES}
        </div>
        <fieldset class="field">
          <legend>Parts</legend>
          <div id="update-parts">${f.partsHtml}</div>
          <p class="hint">Picking Resolved sets this incident's parts back to Operational. You can keep one at Degraded performance if it isn't fully fixed.</p>
          <p class="error">${f.errors.get("parts")}</p>
        </fieldset>
        <div class="row">
          <div class="field">
            <label for="update-next">Next update</label>
            <select id="update-next" name="next">${options(f.next)}</select>
            <p class="error">${f.errors.get("next")}</p>
          </div>
          <div class="field">
            <label for="update-by">Posted by</label>
            <select id="update-by" name="by">${options(f.by)}</select>
            <p class="error">${f.errors.get("by")}</p>
          </div>
        </div>
        <div class="actions"><button class="button">Post update</button></div>
      </form>
    </section>
    `
    }

    <section class="card" aria-labelledby="timeline-h">
      <h2 id="timeline-h">Timeline</h2>
      <p class="muted small">Newest on top. Customers see all of it except the names.</p>
      <ol class="timeline">
        ${i.updates.map(
          (u) => html`
        <li class="stage-${u.stageKey}">
          <p class="stage">${u.stage}</p>
          <p class="message">${u.message}</p>
          <p class="when"><time datetime="${u.iso}">${u.stamp}</time> · ${u.ago} · by ${u.by}${u.promised === "" ? empty : html` · ${u.promised}`}${u.edited ? html` · <span class="edited">edited</span>` : empty}</p>
          ${u.edits.map((e) => html`<p class="was">Before ${e.by} corrected it (${e.stamp}): “${e.previous}”</p>`)}
          <details class="fix"${u.editOpen ? html` open` : empty}>
            <summary>Fix a typo</summary>
            <form class="form" method="post" action="${t.base}/desk/incidents/${i.id}/updates/${u.n}">
              <label for="edit-${u.n}">Corrected text</label>
              <textarea id="edit-${u.n}" name="message" rows="4" maxlength="2000">${u.editText}</textarea>
              <p class="error">${u.editError}</p>
              <div class="row">
                <div class="field">
                  <label for="edit-by-${u.n}">Corrected by</label>
                  <select id="edit-by-${u.n}" name="by">${options(t.editBy)}</select>
                </div>
              </div>
              <p class="hint">Customers will see “edited” next to this update. The earlier text stays here on the desk.</p>
              <div class="actions"><button class="button secondary">Save the correction</button></div>
            </form>
          </details>
        </li>
        `,
        )}
      </ol>
    </section>
  </div>
</div>`;
}

export interface NoteForm {
  message: string;
  newEnd: string;
  by: Opt[];
  errors: Errors;
}

export interface DeskMaintenance extends DeskTop {
  m: MaintenanceView;
  form: NoteForm;
}

/** desk_maintenance.html */
export function deskMaintenance(t: DeskMaintenance): Html {
  const m = t.m;
  const f = t.form;
  return html`<div class="kw desk">
  ${deskTop(t)}
  <div class="wrap stack content">
    ${who(t)}
    <p class="back"><a href="${t.base}/desk${t.meQuery}">← Back to the desk</a></p>

    <section class="card maintenance s-maintenance">
      <header class="card-head">
        <div>
          <p class="eyebrow">${m.phase}${m.by === "" ? empty : html` · announced ${m.announced} by ${m.by}`}</p>
          <h1>${m.title}</h1>
        </div>
        <span class="pill s-maintenance">${m.phaseShort}</span>
      </header>
      <p class="window"><time>${m.when}</time> · ${m.relative}</p>
      <p class="chips"><span class="muted small">Affects</span> ${m.chips.map((c) => html`<span class="chip s-maintenance">${c.name}</span>`)}</p>
      <p class="message">${m.notice}</p>
      <p class="muted small">While the window is open, these parts show “Under maintenance” on the status page by themselves. Afterwards it moves to the past.</p>
    </section>

    <section class="card" aria-labelledby="note-h">
      <h2 id="note-h">Post an update</h2>
      <p class="muted">Runs late? Say so here and set a new end, or open an incident if something broke.</p>
      <form class="form" method="post" action="${t.base}/desk/maintenance/${m.id}/updates">
        <div class="compose">
          <div class="field">
            <label for="note-message">Message for customers</label>
            <textarea id="note-message" name="message" rows="4" maxlength="2000" placeholder="The maintenance takes longer than planned. Kitewell stays read-only until 03:30 UTC.">${f.message}</textarea>
            <p class="error">${f.errors.get("message")}</p>
          </div>
          ${RULES}
        </div>
        <div class="row">
          <div class="field">
            <label for="note-end">New end (UTC), if it moves</label>
            <input id="note-end" type="datetime-local" name="new_end" value="${f.newEnd}">
            <p class="error">${f.errors.get("new_end")}</p>
          </div>
          <div class="field">
            <label for="note-by">Posted by</label>
            <select id="note-by" name="by">${options(f.by)}</select>
            <p class="error">${f.errors.get("by")}</p>
          </div>
        </div>
        <div class="actions"><button class="button">Post update</button></div>
      </form>
    </section>

    ${
      m.updates.length === 0
        ? empty
        : html`
    <section class="card" aria-labelledby="mt-h">
      <h2 id="mt-h">Updates</h2>
      <ol class="timeline">
        ${m.updates.map(
          (u) => html`
        <li>
          <p class="message">${u.message}</p>
          ${u.newEnd === "" ? empty : html`<p class="new-end">${u.newEnd}</p>`}
          <p class="when"><time datetime="${u.iso}">${u.stamp}</time> · ${u.ago} · by ${u.by}</p>
        </li>
        `,
        )}
      </ol>
    </section>
    `
    }
  </div>
</div>`;
}

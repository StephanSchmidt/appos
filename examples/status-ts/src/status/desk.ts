// The desk's handlers: support opens incidents, posts updates, fixes typos,
// sets parts and schedules maintenance. Forms post normally and the browser
// comes back to a page that reads the new state (redirect). Input that
// doesn't parse comes back with what was typed and the message next to the
// field (422). Picking a stage fills in its template through htmx.
//
// There are no logins yet: the desk picks a name, carried in ?me= so it is
// picked once, and every form says who posts.

import { type Request, absoluteUrl, form, notFound, page, param, query, unreadable, urlWith } from "../appos.ts";
import { type Utc, input, stamp } from "../time.ts";
import type { Board } from "./board.ts";
import { boardUc } from "./board_usecase.ts";
import {
  type Author,
  type Id,
  type Incident,
  type PartId,
  type PartStatus,
  type Stage,
  Fail,
  FieldFail,
  PARTS,
  STAGES,
  affects,
  earliestStart,
  incidentStage,
  isOpen,
  isUntouched,
  newMaintenance,
  nextStage,
  openIncident,
  parseAuthor,
  parseId,
  parseMessage,
  parseMoment,
  parseNextUpdate,
  parseOptionalMoment,
  parseStage,
  parseStatus,
  parseTitle,
  postUpdate,
  stageTemplate,
} from "./domain.ts";
import { UNCHANGED, editUc } from "./edit_usecase.ts";
import { noteUc, scheduleUc } from "./maintenance_usecase.ts";
import { openUc } from "./open_usecase.ts";
import { partsUc } from "./parts_usecase.ts";
import { showIncidentUc, showMaintenanceUc } from "./show_usecase.ts";
import * as templates from "./templates.ts";
import { ALREADY_RESOLVED, UNRESOLVED_PARTS, updateUc } from "./update_usecase.ts";
import * as view from "./view.ts";
import { Errors, type Opt, type PartChoice } from "./view.ts";
import { head, now } from "./web.ts";

/** The stages a new incident can start at. */
const OPEN_STAGES: Stage[] = ["investigating", "identified", "monitoring"];

/** Which desk form came back with errors. */
interface Failed {
  form: "open" | "parts" | "maintenance";
  errors: Errors;
}

// ------------------------------------------------------------------ desk

export function index(req: Request): void {
  deskPage(req, undefined);
}

export function open(req: Request): void {
  const at = now(req);
  const errors = new Errors();
  const title = take(errors, "title", parseTitle(field(req, "title")));
  const stage = take(errors, "stage", parseStage(field(req, "stage")));
  const message = take(errors, "message", parseMessage(field(req, "message")));
  const next = take(errors, "next", parseNextUpdate(field(req, "next")));
  const by = take(errors, "by", parseAuthor(field(req, "by")));
  const parts = take(errors, "parts", chosenParts(req));
  if (title === undefined || stage === undefined || message === undefined || next === undefined || by === undefined || parts === undefined) {
    return deskPage(req, { form: "open", errors });
  }
  const cmd = openIncident(title, stage, message, next, by, parts);
  if (cmd instanceof FieldFail) {
    errors.add(cmd.field, cmd.message);
    return deskPage(req, { form: "open", errors });
  }
  const incident = openUc(cmd, at);
  page.redirect(back(req, `/desk/incidents/${incident.id}`, by, "opened"));
}

export function setParts(req: Request): void {
  const errors = new Errors();
  const by = take(errors, "by", parseAuthor(field(req, "by")));
  const parts = take(errors, "parts", chosenParts(req));
  if (by === undefined || parts === undefined) return deskPage(req, { form: "parts", errors });
  const saved = partsUc(parts, by, now(req)) === 0 ? "unchanged" : "parts";
  page.redirect(back(req, "/desk", by, saved));
}

export function schedule(req: Request): void {
  const at = now(req);
  const errors = new Errors();
  const title = take(errors, "title", parseTitle(field(req, "title")));
  const starts = take(errors, "starts", parseMoment(field(req, "starts")));
  const ends = take(errors, "ends", parseMoment(field(req, "ends")));
  const notice = take(errors, "notice", parseMessage(field(req, "notice")));
  const by = take(errors, "by", parseAuthor(field(req, "by")));
  const parts = PARTS.filter((p) => form(req, partField(p)) !== undefined);
  if (title === undefined || starts === undefined || ends === undefined || notice === undefined || by === undefined) {
    return deskPage(req, { form: "maintenance", errors });
  }
  const cmd = newMaintenance(title, parts, starts, ends, notice, by, at);
  if (cmd instanceof FieldFail) {
    errors.add(cmd.field, cmd.message);
    return deskPage(req, { form: "maintenance", errors });
  }
  const m = scheduleUc(cmd, at);
  page.redirect(back(req, `/desk/maintenance/${m.id}`, by, "scheduled"));
}

/** The desk; a form that failed comes back with what was typed. */
function deskPage(req: Request, failed: Failed | undefined): void {
  const board = boardUc(now(req));
  const who = me(req);
  if (failed !== undefined) page.setStatus(422);
  const typed = (which: string): Request | undefined => (failed !== undefined && failed.form === which ? req : undefined);
  const errorsOf = (which: string): Errors => (failed !== undefined && failed.form === which ? failed.errors : new Errors());
  const t: templates.Desk = {
    base: req.base,
    here: "/desk",
    meQuery: meQuery(who),
    nav: nav(who),
    saved: saved(req),
    open: board.openIncidents().map((i) => view.incidentView(i, board.now)),
    resolved: board
      .resolvedIncidents()
      .slice(0, 30)
      .map((i) => view.incidentView(i, board.now)),
    maintenance: [...board.maintenance("in-progress"), ...board.maintenance("upcoming")].map((m) => view.maintenanceView(m, board.now)),
    completed: board
      .maintenance("completed")
      .reverse()
      .slice(0, 10)
      .map((m) => view.maintenanceView(m, board.now)),
    openForm: openForm(typed("open"), who, errorsOf("open")),
    partsForm: partsForm(typed("parts"), who, board, errorsOf("parts")),
    maintForm: maintForm(typed("maintenance"), who, board.now, errorsOf("maintenance")),
  };
  head("Status desk · Kitewell");
  page.setHtml("page", templates.desk(t).text);
}

function openForm(typed: Request | undefined, me: string, errors: Errors): templates.OpenForm {
  const value = (name: string, fallback: string): string => (typed === undefined ? fallback : field(typed, name));
  const stage = value("stage", "investigating");
  return {
    title: value("title", ""),
    stages: view.stages(OPEN_STAGES, stage),
    messageHtml: templates.messageBox("open", value("message", stageTemplate("investigating"))),
    partsHtml: templates.partsBox(
      view.partChoices(
        (p) => value(partField(p), "operational"),
        () => "",
      ),
    ),
    next: view.nextUpdates(value("next", "30")),
    by: view.people(value("by", me)),
    errors,
  };
}

function partsForm(typed: Request | undefined, me: string, board: Board, errors: Errors): templates.PartsForm {
  const value = (p: PartId): string => (typed === undefined ? board.stored(p) : field(typed, partField(p)));
  const note = (p: PartId): string => (board.maintenanceNow(p) === undefined ? "" : "under maintenance now, by itself");
  return {
    partsHtml: templates.partsBox(view.partChoices(value, note)),
    by: view.people(typed === undefined ? me : field(typed, "by")),
    errors,
  };
}

function maintForm(typed: Request | undefined, me: string, at: Utc, errors: Errors): templates.MaintForm {
  const value = (name: string): string => (typed === undefined ? "" : field(typed, name));
  const earliest = earliestStart(at);
  return {
    title: value("title"),
    parts: view.partChecks((p) => typed !== undefined && form(typed, partField(p)) !== undefined),
    starts: value("starts"),
    ends: value("ends"),
    notice: value("notice"),
    by: view.people(typed === undefined ? me : field(typed, "by")),
    earliest: stamp(earliest),
    earliestInput: input(earliest),
    errors,
  };
}

/** The stage select changed (htmx): fill in that stage's template unless
 * the box has the desk's own words, and for an update set the parts the
 * way the stage usually leaves them. */
export function compose(req: Request): void {
  const stage = parseStage(query(req, "stage") ?? "");
  if (stage instanceof Fail) {
    page.setStatus(422);
    return;
  }
  const which = query(req, "form");
  if (which !== "open" && which !== "update") return notFound();
  if (isUntouched(query(req, "message") ?? "")) {
    page.setHtml(`${which}-message`, templates.messageBox(which, stageTemplate(stage)).text);
  }
  if (which === "update") {
    const id = parseId(query(req, "incident") ?? "");
    if (id === undefined) return notFound();
    const shown = showIncidentUc(id);
    if (shown.ok) {
      const board = boardUc(now(req));
      page.setHtml("update-parts", templates.partsBox(updateParts(stage, shown.incident, board, undefined)).text);
      return;
    }
    switch (shown.error) {
      case "not-found":
        return notFound();
      case "unreadable":
        return unreadable(shown);
    }
  }
}

// -------------------------------------------------------------- incident

export function incident(req: Request): void {
  const id = parseId(param(req, "id") ?? "");
  if (id === undefined) return notFound();
  incidentPage(req, id, undefined, undefined);
}

export function postUpdateHandler(req: Request): void {
  const id = parseId(param(req, "id") ?? "");
  if (id === undefined) return notFound();
  const errors = new Errors();
  const stage = take(errors, "stage", parseStage(field(req, "stage")));
  const message = take(errors, "message", parseMessage(field(req, "message")));
  const next = take(errors, "next", parseNextUpdate(field(req, "next")));
  const by = take(errors, "by", parseAuthor(field(req, "by")));
  const parts = take(errors, "parts", chosenParts(req));
  if (stage === undefined || message === undefined || next === undefined || by === undefined || parts === undefined) {
    page.setStatus(422);
    return incidentPage(req, id, errors, undefined);
  }
  const cmd = postUpdate(stage, message, next, by, parts);
  const result = updateUc(id, cmd, now(req));
  if (result.ok) {
    page.redirect(back(req, `/desk/incidents/${id}`, by, isOpen(result.incident) ? "posted" : "resolved"));
    return;
  }
  switch (result.error) {
    case "not-found":
      return notFound();
    case "already-resolved":
      errors.add("stage", ALREADY_RESOLVED);
      page.setStatus(409);
      return incidentPage(req, id, errors, undefined);
    case "unresolved-parts":
      errors.add("parts", UNRESOLVED_PARTS);
      page.setStatus(422);
      return incidentPage(req, id, errors, undefined);
    case "unreadable":
      return unreadable(result);
  }
}

export function editUpdate(req: Request): void {
  const id = parseId(param(req, "id") ?? "");
  if (id === undefined) return notFound();
  const n = parseIndex(param(req, "n") ?? "");
  if (n === undefined) return notFound();
  const errors = new Errors();
  const message = take(errors, "message", parseMessage(field(req, "message")));
  const by = take(errors, "by", parseAuthor(field(req, "by")));
  if (message === undefined || by === undefined) {
    page.setStatus(422);
    return incidentPage(req, id, undefined, { n, errors });
  }
  const result = editUc(id, n, message, by, now(req));
  if (result.ok) {
    page.redirect(back(req, `/desk/incidents/${id}`, by, "edited"));
    return;
  }
  switch (result.error) {
    case "not-found":
    case "no-such-update":
      return notFound();
    case "unchanged":
      errors.add("message", UNCHANGED);
      page.setStatus(422);
      return incidentPage(req, id, undefined, { n, errors });
    case "unreadable":
      return unreadable(result);
  }
}

/** The typo form of update n came back. */
interface EditFailed {
  n: number;
  errors: Errors;
}

/** An incident on the desk. `updateErrors`: the update form came back;
 * `edit`: the typo form of update n came back. */
function incidentPage(req: Request, id: Id, updateErrors: Errors | undefined, edit: EditFailed | undefined): void {
  const shown = showIncidentUc(id);
  if (!shown.ok) {
    switch (shown.error) {
      case "not-found":
        return notFound();
      case "unreadable":
        return unreadable(shown);
    }
  }
  const incident = shown.incident;
  const board = boardUc(now(req));
  const who = me(req);
  const i = view.incidentView(incident, board.now);
  if (edit !== undefined) {
    for (const u of i.updates) {
      if (u.n !== edit.n) continue;
      u.editOpen = true;
      u.editText = field(req, "message");
      u.editError = [edit.errors.get("message"), edit.errors.get("by")].filter((e) => e !== "").join(" ");
    }
  }
  const typed = updateErrors === undefined ? undefined : req;
  const t: templates.DeskIncident = {
    base: req.base,
    here: `/desk/incidents/${id}`,
    meQuery: meQuery(who),
    nav: nav(who),
    saved: saved(req),
    publicUrl: absoluteUrl(req, `/incidents/${id}`),
    form: updateForm(typed, incident, board, who, updateErrors ?? new Errors()),
    editBy: view.people(edit === undefined ? who : field(req, "by")),
    i,
  };
  head(`${incident.title} · Status desk`);
  page.setHtml("page", templates.deskIncident(t).text);
}

function updateForm(typed: Request | undefined, incident: Incident, board: Board, me: string, errors: Errors): templates.UpdateForm {
  const suggested = nextStage(incidentStage(incident));
  const value = (name: string, fallback: string): string => (typed === undefined ? fallback : field(typed, name));
  const stage = value("stage", suggested);
  const parsed = parseStage(stage);
  return {
    stages: view.stages(STAGES, stage),
    messageHtml: templates.messageBox("update", value("message", stageTemplate(suggested))),
    partsHtml: templates.partsBox(updateParts(parsed instanceof Fail ? suggested : parsed, incident, board, typed)),
    next: view.nextUpdates(value("next", suggested === "resolved" ? "" : "30")),
    by: view.people(value("by", me)),
    errors,
  };
}

/** The parts of the update form: as they are, or what was typed; picking
 * Resolved sets this incident's parts back to Operational. */
function updateParts(stage: Stage, incident: Incident, board: Board, typed: Request | undefined): PartChoice[] {
  const value = (p: PartId): string => {
    if (typed !== undefined) return field(typed, partField(p));
    if (stage === "resolved" && affects(incident, p)) return "operational";
    return board.stored(p);
  };
  const note = (p: PartId): string => {
    if (affects(incident, p)) return "in this incident";
    if (board.maintenanceNow(p) !== undefined) return "under maintenance now";
    return "";
  };
  return view.partChoices(value, note);
}

// ----------------------------------------------------------- maintenance

export function maintenance(req: Request): void {
  const id = parseId(param(req, "id") ?? "");
  if (id === undefined) return notFound();
  maintenancePage(req, id, undefined);
}

export function postMaintenanceUpdate(req: Request): void {
  const id = parseId(param(req, "id") ?? "");
  if (id === undefined) return notFound();
  const errors = new Errors();
  const message = take(errors, "message", parseMessage(field(req, "message")));
  const newEnd = take(errors, "new_end", parseOptionalMoment(field(req, "new_end")));
  const by = take(errors, "by", parseAuthor(field(req, "by")));
  if (message === undefined || newEnd === undefined || by === undefined) {
    page.setStatus(422);
    return maintenancePage(req, id, errors);
  }
  const result = noteUc(id, message, newEnd, by, now(req));
  if (result.ok) {
    page.redirect(back(req, `/desk/maintenance/${id}`, by, "noted"));
    return;
  }
  switch (result.error) {
    case "not-found":
      return notFound();
    case "refused":
      errors.add("new_end", result.message);
      page.setStatus(422);
      return maintenancePage(req, id, errors);
    case "unreadable":
      return unreadable(result);
  }
}

function maintenancePage(req: Request, id: Id, errors: Errors | undefined): void {
  const shown = showMaintenanceUc(id);
  if (!shown.ok) {
    switch (shown.error) {
      case "not-found":
        return notFound();
      case "unreadable":
        return unreadable(shown);
    }
  }
  const m = shown.maintenance;
  const who = me(req);
  const value = (name: string): string => (errors === undefined ? "" : field(req, name));
  const t: templates.DeskMaintenance = {
    base: req.base,
    here: `/desk/maintenance/${id}`,
    meQuery: meQuery(who),
    nav: nav(who),
    saved: saved(req),
    m: view.maintenanceView(m, now(req)),
    form: {
      message: value("message"),
      newEnd: value("new_end"),
      by: view.people(errors === undefined ? who : field(req, "by")),
      errors: errors ?? new Errors(),
    },
  };
  head(`${m.title} · Status desk`);
  page.setHtml("page", templates.deskMaintenance(t).text);
}

// --------------------------------------------------------------- helpers

/** A form field, "" if the form didn't send it. */
function field(req: Request, name: string): string {
  return form(req, name) ?? "";
}

/** The parsed value, or undefined with the problem noted for the field. */
function take<T>(errors: Errors, name: string, result: T | Fail): T | undefined {
  if (result instanceof Fail) {
    errors.add(name, result.message);
    return undefined;
  }
  return result;
}

/** An update's position in a URL, like Rust's usize parse: digits, with an
 * optional "+". */
function parseIndex(text: string): number | undefined {
  const digits = text.startsWith("+") ? text.slice(1) : text;
  if (digits === "") return undefined;
  for (let i = 0; i < digits.length; i++) {
    const c = digits.charCodeAt(i);
    if (c < 48 || c > 57) return undefined;
  }
  return Number.parseInt(digits, 10);
}

function partField(part: PartId): string {
  return `part-${part}`;
}

/** The parts' statuses the form sent; a part the form didn't send keeps
 * its status. */
function chosenParts(req: Request): PartStatus[] | Fail {
  const parts: PartStatus[] = [];
  for (const part of PARTS) {
    const value = form(req, partField(part));
    if (value === undefined) continue;
    const status = parseStatus(value);
    if (status instanceof Fail) return status;
    parts.push({ part, status });
  }
  return parts;
}

/** Who is on the desk: ?me=, or the name a form just sent. */
function me(req: Request): string {
  const key = query(req, "me") ?? form(req, "by");
  if (key === undefined) return "";
  const author = parseAuthor(key);
  return author instanceof Fail ? "" : author;
}

function meQuery(who: string): string {
  return who === "" ? "" : `?me=${who}`;
}

/** The names to pick from at the top of the desk. */
function nav(who: string): Opt[] {
  return view.people(who).filter((o) => o.value !== "");
}

/** Where to go after a form: the page, with who posted and what happened. */
function back(req: Request, path: string, by: Author, saved: string): string {
  return urlWith(req, path, [
    ["me", by],
    ["saved", saved],
  ]);
}

/** The line that says the last form worked. */
function saved(req: Request): string {
  switch (query(req, "saved") ?? "") {
    case "opened":
      return "Incident opened. Customers see it on the status page now.";
    case "posted":
      return "Update posted. Customers see it now.";
    case "resolved":
      return "Incident resolved. It moved to “Past incidents” on the status page.";
    case "edited":
      return "Text corrected. Customers see “edited” next to it.";
    case "parts":
      return "Part statuses saved. Customers see them now.";
    case "unchanged":
      return "Nothing changed: the parts already had these statuses.";
    case "scheduled":
      return "Maintenance scheduled. Customers see it as upcoming maintenance.";
    case "noted":
      return "Maintenance update posted. Customers see it now.";
    default:
      return "";
  }
}

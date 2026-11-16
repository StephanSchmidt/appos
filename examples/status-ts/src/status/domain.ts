// The status page's data and its rules: plain TypeScript, no IO. The parts
// customers see, how bad things are (Status), incidents and their updates,
// planned maintenance, who is on the desk, and the checks of what the desk
// types (parseTitle, parseMessage, ...).
//
// The stored types (Incident, Change, Maintenance) have the JSON's field
// names, so db.ts stores them as they are.

import { DAY, HOUR, type Utc, parseInput, stamp } from "../time.ts";
import { newId } from "../appos.ts";

/** Why some input was refused, in words for the desk. */
export class Fail {
  readonly message: string;
  constructor(message: string) {
    this.message = message;
  }
}

// ----------------------------------------------------------------- parts

/** A part of Kitewell that customers see on the page; also its name in
 * forms, URLs and the stored JSON. */
export type PartId = "web-app" | "ios" | "android" | "api" | "slack" | "google-calendar" | "teams" | "email" | "push";

export const PARTS: PartId[] = ["web-app", "ios", "android", "api", "slack", "google-calendar", "teams", "email", "push"];

/** Its name inside its group: "iOS" under "Mobile apps". */
export function partName(p: PartId): string {
  switch (p) {
    case "web-app":
      return "Kitewell web app";
    case "ios":
      return "iOS";
    case "android":
      return "Android";
    case "api":
      return "API";
    case "slack":
      return "Slack";
    case "google-calendar":
      return "Google Calendar";
    case "teams":
      return "Microsoft Teams";
    case "email":
      return "Email";
    case "push":
      return "Push notifications";
  }
}

/** Its name on its own, as in "Affects: iOS app". */
export function partFullName(p: PartId): string {
  switch (p) {
    case "web-app":
      return "Web app";
    case "ios":
      return "iOS app";
    case "android":
      return "Android app";
    case "api":
      return "API";
    case "slack":
      return "Slack integration";
    case "google-calendar":
      return "Google Calendar integration";
    case "teams":
      return "Microsoft Teams integration";
    case "email":
      return "Email notifications";
    case "push":
      return "Push notifications";
  }
}

/** The parts, grouped the way customers think of them. A group of one is
 * shown as the part alone. */
export interface Group {
  name: string;
  parts: PartId[];
}

export const GROUPS: Group[] = [
  { name: "Kitewell web app", parts: ["web-app"] },
  { name: "Mobile apps", parts: ["ios", "android"] },
  { name: "API", parts: ["api"] },
  { name: "Integrations", parts: ["slack", "google-calendar", "teams"] },
  { name: "Notifications", parts: ["email", "push"] },
];

// ---------------------------------------------------------------- status

/** How a part is doing; `severity` orders them from good to bad. */
export type Status = "operational" | "maintenance" | "degraded" | "partial" | "major";

export const STATUSES: Status[] = ["operational", "degraded", "partial", "major", "maintenance"];

/** What the desk can set. Maintenance comes from the schedule only. */
export const STATUS_CHOICES: Status[] = ["operational", "degraded", "partial", "major"];

/** 0 for operational up to 4 for a major outage; also the index into a
 * day's seconds. */
export function severity(s: Status): number {
  switch (s) {
    case "operational":
      return 0;
    case "maintenance":
      return 1;
    case "degraded":
      return 2;
    case "partial":
      return 3;
    case "major":
      return 4;
  }
}

/** The worse of two. */
export function worse(a: Status, b: Status): Status {
  return severity(b) > severity(a) ? b : a;
}

/** The worst of some, operational for none. */
export function worst(statuses: Status[]): Status {
  let w: Status = "operational";
  for (const s of statuses) w = worse(w, s);
  return w;
}

export function statusLabel(s: Status): string {
  switch (s) {
    case "operational":
      return "Operational";
    case "maintenance":
      return "Under maintenance";
    case "degraded":
      return "Degraded performance";
    case "partial":
      return "Partial outage";
    case "major":
      return "Major outage";
  }
}

/** Only partial and major outages count as down for the uptime. */
export function isDown(s: Status): boolean {
  return s === "partial" || s === "major";
}

/** A status the desk picked; maintenance isn't one of them. */
export function parseStatus(key: string): Status | Fail {
  for (const s of STATUS_CHOICES) if (s === key) return s;
  return new Fail("Pick a status from the list.");
}

/** The one sentence at the top of the page. */
export type Overall = "all-good" | "maintenance" | "problems" | "major";

export function overallOf(w: Status): Overall {
  switch (w) {
    case "operational":
      return "all-good";
    case "maintenance":
      return "maintenance";
    case "degraded":
    case "partial":
      return "problems";
    case "major":
      return "major";
  }
}

export function overallSentence(o: Overall): string {
  switch (o) {
    case "all-good":
      return "All systems operational";
    case "maintenance":
      return "Planned maintenance in progress";
    case "problems":
      return "Some systems are having problems";
    case "major":
      return "Major outage";
  }
}

/** The status whose colour it takes. */
export function overallStatus(o: Overall): Status {
  switch (o) {
    case "all-good":
      return "operational";
    case "maintenance":
      return "maintenance";
    case "problems":
      return "partial";
    case "major":
      return "major";
  }
}

// ----------------------------------------------------------------- stage

/** Where an incident is. */
export type Stage = "investigating" | "identified" | "monitoring" | "resolved";

export const STAGES: Stage[] = ["investigating", "identified", "monitoring", "resolved"];

export function stageLabel(s: Stage): string {
  switch (s) {
    case "investigating":
      return "Investigating";
    case "identified":
      return "Identified";
    case "monitoring":
      return "Monitoring";
    case "resolved":
      return "Resolved";
  }
}

export function parseStage(key: string): Stage | Fail {
  for (const s of STAGES) if (s === key) return s;
  return new Fail("Pick a stage.");
}

/** The text a stressed agent starts from; the part in brackets is theirs
 * to replace (parseMessage refuses it until they do). */
export function stageTemplate(s: Stage): string {
  switch (s) {
    case "investigating":
      return "We're looking into reports that [what customers notice]. We'll post an update within 30 minutes.";
    case "identified":
      return "We've found the cause of [the problem] and are working on a fix.";
    case "monitoring":
      return "A fix is in place and we're watching the results. Things should be back to normal for most customers.";
    case "resolved":
      return "This is resolved: [what] works normally again. We're sorry for the trouble.";
  }
}

/** The stage an update usually moves to. */
export function nextStage(s: Stage): Stage {
  switch (s) {
    case "investigating":
      return "identified";
    case "identified":
      return "monitoring";
    case "monitoring":
    case "resolved":
      return "resolved";
  }
}

/** True if the text is empty or one of the templates as it comes: then
 * picking another stage may replace it without losing anyone's words. */
export function isUntouched(text: string): boolean {
  const t = text.trim();
  return t === "" || STAGES.some((s) => stageTemplate(s) === t);
}

// ---------------------------------------------------------------- author

/** Who is on the desk. There are no logins yet: the desk picks a name. */
export type Author = "amara" | "jonas" | "mei" | "lucas";

export const AUTHORS: Author[] = ["amara", "jonas", "mei", "lucas"];

export function authorName(a: Author): string {
  switch (a) {
    case "amara":
      return "Amara Okafor";
    case "jonas":
      return "Jonas Weber";
    case "mei":
      return "Mei Tanaka";
    case "lucas":
      return "Lucas Silva";
  }
}

export function parseAuthor(key: string): Author | Fail {
  for (const a of AUTHORS) if (a === key) return a;
  return new Fail("Pick your name.");
}

// ----------------------------------------------------------------- input

export const TITLE_MAX = 120;
export const MESSAGE_MAX = 2000;

/** Characters, as Rust counts them: code points, not UTF-16 units. */
function chars(s: string): number {
  let n = 0;
  for (const _ of s) n++;
  return n;
}

/** An incident's or maintenance's title, in customer words, on one line. */
export function parseTitle(input: string): string | Fail {
  const title = oneLine(input);
  if (title === "") return new Fail("Give it a title in customer words, like “Some users can't log in”.");
  if (chars(title) > TITLE_MAX) return new Fail(`Please keep the title to ${TITLE_MAX} characters.`);
  return title;
}

/** What customers read: an update's message or what a maintenance changes
 * for them. Never with a template's part in brackets still in it. */
export function parseMessage(input: string): string | Fail {
  const text = trimRust(input.replaceAll("\r\n", "\n"));
  if (text === "") return new Fail("Write the message for customers.");
  if (chars(text) > MESSAGE_MAX) return new Fail(`Please keep it to ${MESSAGE_MAX} characters.`);
  const p = placeholder(text);
  if (p !== undefined) return new Fail(`Replace “${p}” with your own words.`);
  return text;
}

/** The first "[...]" in a text. */
function placeholder(text: string): string | undefined {
  const start = text.indexOf("[");
  if (start < 0) return undefined;
  const end = text.indexOf("]", start);
  if (end < 0) return undefined;
  return text.slice(start, end + 1);
}

/** "Next update in 30 minutes": none, 30 or 60. */
export function parseNextUpdate(input: string): number | null | Fail {
  if (input === "" || input === "none") return null;
  if (input === "30") return 30;
  if (input === "60") return 60;
  return new Fail("Pick 30 or 60 minutes, or no time.");
}

/** A date and time the desk typed, in UTC. */
export function parseMoment(input: string): Utc | Fail {
  if (trimRust(input) === "") return new Fail("Enter the date and time (UTC).");
  const t = parseInput(input);
  return t === undefined ? new Fail("Enter the date and time like 2026-11-22 02:00 (UTC).") : t;
}

/** An optional date and time: empty is none. */
export function parseOptionalMoment(input: string): Utc | null | Fail {
  if (trimRust(input) === "") return null;
  return parseMoment(input);
}

/** Rust's char::is_whitespace: Unicode White_Space. */
function isSpace(c: string): boolean {
  const n = c.codePointAt(0) ?? 0;
  return (
    (n >= 0x09 && n <= 0x0d) ||
    n === 0x20 ||
    n === 0x85 ||
    n === 0xa0 ||
    n === 0x1680 ||
    (n >= 0x2000 && n <= 0x200a) ||
    n === 0x2028 ||
    n === 0x2029 ||
    n === 0x202f ||
    n === 0x205f ||
    n === 0x3000
  );
}

/** Rust's str::trim. JS's trim also strips U+FEFF, which Rust keeps. */
function trimRust(s: string): string {
  const cs = [...s];
  let a = 0;
  let b = cs.length;
  while (a < b && isSpace(cs[a])) a++;
  while (b > a && isSpace(cs[b - 1])) b--;
  return cs.slice(a, b).join("");
}

/** The words, joined by single spaces (Rust's split_whitespace). */
function oneLine(input: string): string {
  const words: string[] = [];
  let word = "";
  for (const c of input) {
    if (isSpace(c)) {
      if (word !== "") words.push(word);
      word = "";
    } else {
      word += c;
    }
  }
  if (word !== "") words.push(word);
  return words.join(" ");
}

// ------------------------------------------------------------------- ids

/** An id: a UUID v7, lowercase with hyphens, so ids sort by when they
 * were made. */
export type Id = string;

/** An id from a URL, written the way it is stored; undefined if the text
 * isn't a UUID. Like Rust's Uuid::try_parse: hyphenated, plain hex, in
 * braces or as a urn, any case. */
export function parseId(text: string): Id | undefined {
  let hex: string;
  if (text.length === 36) {
    if (text[8] !== "-" || text[13] !== "-" || text[18] !== "-" || text[23] !== "-") return undefined;
    hex = text.slice(0, 8) + text.slice(9, 13) + text.slice(14, 18) + text.slice(19, 23) + text.slice(24);
  } else if (text.length === 32) {
    hex = text;
  } else if (text.length === 38 && text.startsWith("{") && text.endsWith("}")) {
    return parseId(text.slice(1, 37));
  } else if (text.length === 45 && text.startsWith("urn:uuid:")) {
    return parseId(text.slice(9));
  } else {
    return undefined;
  }
  for (let i = 0; i < hex.length; i++) {
    const c = hex.charCodeAt(i);
    const ok = (c >= 48 && c <= 57) || (c >= 65 && c <= 70) || (c >= 97 && c <= 102);
    if (!ok) return undefined;
  }
  const h = hex.toLowerCase();
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

// -------------------------------------------------------------- incident

/** Something that went wrong, in customer words, with its updates. */
export interface Incident {
  id: Id;
  title: string;
  opened_at: Utc;
  resolved_at: Utc | null;
  /** The parts it affects: how they are now and the worst they were. */
  impact: Impact[];
  /** Oldest first. Updates are never deleted. */
  updates: Update[];
}

export interface Impact {
  part: PartId;
  now: Status;
  worst: Status;
}

export interface Update {
  stage: Stage;
  message: string;
  at: Utc;
  /** For the desk only; customers never see names. */
  by: Author;
  /** Minutes until the next update, if the desk promised one. */
  next_update: number | null;
  /** Earlier texts, oldest first, when someone fixed a typo. */
  edits: Edit[];
}

export interface Edit {
  previous: string;
  at: Utc;
  by: Author;
}

/** A part and the status the desk chose for it. */
export interface PartStatus {
  part: PartId;
  status: Status;
}

/** A new incident, as the desk opened it. */
export interface OpenIncident {
  title: string;
  stage: Stage;
  message: string;
  nextUpdate: number | null;
  by: Author;
  /** The affected parts and how badly; none of them Operational. */
  parts: PartStatus[];
}

/** Checks a new incident; the field the problem is in comes with it. */
export function openIncident(
  title: string,
  stage: Stage,
  message: string,
  nextUpdate: number | null,
  by: Author,
  chosen: PartStatus[],
): OpenIncident | FieldFail {
  if (stage === "resolved") return fieldFail("stage", "A new incident can't start as resolved.");
  const parts = chosen.filter((p) => p.status !== "operational");
  if (parts.length === 0) return fieldFail("parts", "Pick at least one affected part and how badly it's affected.");
  return { title, stage, message, nextUpdate, by, parts };
}

/** An update the desk posts to an open incident. Resolved is an update too. */
export interface PostUpdate {
  stage: Stage;
  message: string;
  nextUpdate: number | null;
  by: Author;
  /** Every part, with the status the desk chose for it. */
  parts: PartStatus[];
}

export function postUpdate(stage: Stage, message: string, nextUpdate: number | null, by: Author, parts: PartStatus[]): PostUpdate {
  // Nothing comes after resolved.
  return { stage, message, nextUpdate: stage === "resolved" ? null : nextUpdate, by, parts };
}

/** A new incident from the desk's form. */
export function newIncident(cmd: OpenIncident, now: Utc): Incident {
  const incident: Incident = {
    id: newId(),
    title: cmd.title,
    opened_at: now,
    resolved_at: null,
    impact: [],
    updates: [{ stage: cmd.stage, message: cmd.message, at: now, by: cmd.by, next_update: cmd.nextUpdate, edits: [] }],
  };
  record(incident, cmd.parts);
  return incident;
}

export function isOpen(i: Incident): boolean {
  return i.resolved_at === null;
}

export function latest(i: Incident): Update | undefined {
  return i.updates.length === 0 ? undefined : i.updates[i.updates.length - 1];
}

export function incidentStage(i: Incident): Stage {
  const u = latest(i);
  return u === undefined ? "investigating" : u.stage;
}

export function affects(i: Incident, part: PartId): boolean {
  return i.impact.some((x) => x.part === part);
}

/** When it ended, or now while it is open. */
export function ended(i: Incident, now: Utc): Utc {
  return i.resolved_at ?? now;
}

/** The worst status it causes now. */
export function worstNow(i: Incident): Status {
  return worst(i.impact.map((x) => x.now));
}

/** Posts an update. `current` is each part's status right now; returns the
 * parts whose status changes, for the history. */
export function post(i: Incident, cmd: PostUpdate, current: (part: PartId) => Status, now: Utc): PartStatus[] {
  const changes = cmd.parts.filter((p) => current(p.part) !== p.status);
  // Keep this incident's impact current even if another desk action has
  // already put a part in the chosen state. Unchanged, unrelated outages
  // belong to their own incidents.
  record(i, i.impact.map((p) => ({ part: p.part, status: cmd.parts.find((chosen) => chosen.part === p.part)?.status ?? current(p.part) })));
  record(i, changes);
  if (cmd.stage === "resolved") i.resolved_at = now;
  i.updates.push({ stage: cmd.stage, message: cmd.message, at: now, by: cmd.by, next_update: cmd.nextUpdate, edits: [] });
  return changes;
}

/** Fixes a typo in update `n` (0 is the first). The old text is kept for
 * the desk; customers see "edited". */
export function edit(i: Incident, n: number, text: string, by: Author, now: Utc): "no-such-update" | "unchanged" | undefined {
  if (n >= i.updates.length) return "no-such-update";
  const update = i.updates[n];
  if (update.message === text) return "unchanged";
  update.edits.push({ previous: update.message, at: now, by });
  update.message = text;
  return undefined;
}

/** Notes new statuses in the impact: a part it didn't affect yet joins
 * when it gets worse than Operational. */
function record(i: Incident, changes: PartStatus[]): void {
  for (const { part, status } of changes) {
    const impact = i.impact.find((x) => x.part === part);
    if (impact !== undefined) {
      impact.now = status;
      impact.worst = worse(impact.worst, status);
    } else if (status !== "operational") {
      i.impact.push({ part, now: status, worst: status });
    }
  }
}

/** A part's status changed: the history behind the 90 days and the uptime. */
export interface Change {
  part: PartId;
  status: Status;
  at: Utc;
  by: Author | null;
  incident: Id | null;
}

// ----------------------------------------------------------- maintenance

/** Maintenance is announced at least this many days ahead. */
export const NOTICE_DAYS = 3;
/** The longest window, in hours. */
export const WINDOW_MAX_HOURS = 24;

export interface Maintenance {
  id: Id;
  title: string;
  parts: PartId[];
  starts: Utc;
  ends: Utc;
  /** What customers will notice. */
  notice: string;
  by: Author | null;
  announced_at: Utc;
  /** Oldest first. */
  updates: MaintenanceUpdate[];
}

export interface MaintenanceUpdate {
  message: string;
  at: Utc;
  by: Author;
  /** Set when the window moved its end, e.g. because it runs late. */
  new_end: Utc | null;
}

export type Phase = "upcoming" | "in-progress" | "completed";

export function phase(m: Maintenance, now: Utc): Phase {
  if (now < m.starts) return "upcoming";
  if (now < m.ends) return "in-progress";
  return "completed";
}

export function maintenanceAffects(m: Maintenance, part: PartId): boolean {
  return m.parts.includes(part);
}

/** Posts an update; a new end moves the window. */
export function postNote(m: Maintenance, note: MaintenanceNote, now: Utc): void {
  if (note.newEnd !== null) m.ends = note.newEnd;
  m.updates.push({ message: note.message, at: now, by: note.by, new_end: note.newEnd });
}

/** A problem with a form, and the field it is in. */
export class FieldFail {
  readonly field: string;
  readonly message: string;
  constructor(field: string, message: string) {
    this.field = field;
    this.message = message;
  }
}

function fieldFail(field: string, message: string): FieldFail {
  return new FieldFail(field, message);
}

/** Maintenance as the desk schedules it. */
export interface NewMaintenance {
  title: string;
  parts: PartId[];
  starts: Utc;
  ends: Utc;
  notice: string;
  by: Author;
}

export function newMaintenance(
  title: string,
  parts: PartId[],
  starts: Utc,
  ends: Utc,
  notice: string,
  by: Author,
  now: Utc,
): NewMaintenance | FieldFail {
  if (parts.length === 0) return fieldFail("parts", "Pick the parts it affects.");
  const earliest = earliestStart(now);
  if (starts < earliest) return fieldFail("starts", `Announce maintenance at least 3 days ahead: start at ${stamp(earliest)} or later.`);
  if (ends <= starts) return fieldFail("ends", "The end must be after the start.");
  if (ends - starts > WINDOW_MAX_HOURS * HOUR) return fieldFail("ends", `A maintenance window can be at most ${WINDOW_MAX_HOURS} hours.`);
  return { title, parts, starts, ends, notice, by };
}

export function intoMaintenance(cmd: NewMaintenance, now: Utc): Maintenance {
  return {
    id: newId(),
    title: cmd.title,
    parts: cmd.parts,
    starts: cmd.starts,
    ends: cmd.ends,
    notice: cmd.notice,
    by: cmd.by,
    announced_at: now,
    updates: [],
  };
}

/** The earliest start the desk may schedule: three days ahead, to the
 * minute. */
export function earliestStart(now: Utc): Utc {
  const nextMinute = Math.trunc((now + 59) / 60) * 60;
  return nextMinute + NOTICE_DAYS * DAY;
}

/** An update to a maintenance, e.g. because it runs late. */
export interface MaintenanceNote {
  message: string;
  newEnd: Utc | null;
  by: Author;
}

export function maintenanceNote(message: string, newEnd: Utc | null, by: Author, starts: Utc, now: Utc): MaintenanceNote | Fail {
  if (newEnd !== null) {
    if (newEnd <= now) return new Fail("The new end must be in the future.");
    if (newEnd <= starts) return new Fail("The new end must be after the start.");
  }
  return { message, newEnd, by };
}

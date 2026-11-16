// What the pages show, worked out from the data: the words, times and CSS
// classes the templates (templates.ts) print. Plain TypeScript, no IO.

import { type Month, type Utc, ago, clock, date, dayStart, duration, iso, month, monthName, previousMonth, shortDate, stamp, weekdayDate } from "../time.ts";
import type { Board, Day } from "./board.ts";
import {
  type Incident,
  type Maintenance,
  type PartId,
  type Stage,
  type Status,
  AUTHORS,
  GROUPS,
  PARTS,
  STATUS_CHOICES,
  authorName,
  ended,
  incidentStage,
  isOpen,
  latest,
  partFullName,
  partName,
  phase,
  severity,
  stageLabel,
  statusLabel,
  worst,
  worstNow,
} from "./domain.ts";

/** A part and how it is affected: "Email notifications · Partial outage". */
export interface Chip {
  name: string;
  label: string;
  key: string;
}

function chip(part: PartId, status: Status): Chip {
  return { name: partFullName(part), label: statusLabel(status), key: status };
}

export interface UpdateView {
  /** Its position, oldest first: the URL to fix a typo in it. */
  n: number;
  stage: string;
  stageKey: string;
  message: string;
  stamp: string;
  iso: string;
  ago: string;
  edited: boolean;
  /** For the desk only. */
  by: string;
  edits: EditView[];
  promised: string;
  /** The typo form: its text, its error, and whether it is open. */
  editText: string;
  editError: string;
  editOpen: boolean;
}

export interface EditView {
  previous: string;
  stamp: string;
  by: string;
}

export interface IncidentView {
  id: string;
  title: string;
  open: boolean;
  /** The latest stage, or "Resolved". */
  state: string;
  /** The colour: the worst status it causes now, or caused. */
  stateKey: string;
  chips: Chip[];
  /** Newest first. */
  updates: UpdateView[];
  nextUpdate: string | undefined;
  when: string;
  lasted: string;
  latestAgo: string;
}

export function incidentView(incident: Incident, now: Utc): IncidentView {
  const open = isOpen(incident);
  const chips = incident.impact.map((i) => chip(i.part, open ? i.now : i.worst));
  const updates: UpdateView[] = [];
  for (let n = incident.updates.length - 1; n >= 0; n--) {
    const u = incident.updates[n];
    updates.push({
      n,
      stage: stageLabel(u.stage),
      stageKey: u.stage,
      message: u.message,
      stamp: stamp(u.at),
      iso: iso(u.at),
      ago: ago(u.at, now),
      edited: u.edits.length > 0,
      by: authorName(u.by),
      edits: u.edits.map((e) => ({ previous: e.previous, stamp: stamp(e.at), by: authorName(e.by) })),
      promised: u.next_update === null ? "" : `next update promised in ${u.next_update} minutes`,
      editText: u.message,
      editError: "",
      editOpen: false,
    });
  }
  const last = latest(incident);
  let nextUpdate: string | undefined = undefined;
  if (open && last !== undefined && last.next_update !== null) {
    const due = last.at + last.next_update * 60;
    nextUpdate = due > now ? `Next update by ${stamp(due)} (${ago(due, now)})` : `Next update expected since ${stamp(due)}`;
  }
  const end = ended(incident, now);
  return {
    id: incident.id,
    title: incident.title,
    open,
    state: open ? stageLabel(incidentStage(incident)) : stageLabel("resolved"),
    stateKey: open ? worstNow(incident) : "operational",
    chips,
    updates,
    nextUpdate,
    when: open ? `Since ${stamp(incident.opened_at)}` : range(incident.opened_at, end),
    lasted: duration(end - incident.opened_at),
    latestAgo: last === undefined ? "" : ago(last.at, now),
  };
}

export interface MaintenanceView {
  id: string;
  title: string;
  /** "Upcoming maintenance", "Maintenance in progress", ... */
  phase: string;
  phaseShort: string;
  when: string;
  relative: string;
  chips: Chip[];
  notice: string;
  /** Newest first. */
  updates: NoteView[];
  lasted: string;
  /** For the desk only. */
  by: string;
  announced: string;
}

export interface NoteView {
  message: string;
  stamp: string;
  iso: string;
  ago: string;
  newEnd: string;
  by: string;
}

export function maintenanceView(m: Maintenance, now: Utc): MaintenanceView {
  let label: string;
  let short: string;
  let relative: string;
  switch (phase(m, now)) {
    case "upcoming":
      label = "Upcoming maintenance";
      short = "Upcoming";
      relative = `starts ${ago(m.starts, now)}`;
      break;
    case "in-progress":
      label = "Maintenance in progress";
      short = "Under maintenance";
      relative = `ends ${ago(m.ends, now)}`;
      break;
    case "completed":
      label = "Completed maintenance";
      short = "Completed";
      relative = `ended ${ago(m.ends, now)}`;
      break;
  }
  const updates: NoteView[] = [];
  for (let n = m.updates.length - 1; n >= 0; n--) {
    const u = m.updates[n];
    updates.push({
      message: u.message,
      stamp: stamp(u.at),
      iso: iso(u.at),
      ago: ago(u.at, now),
      newEnd: u.new_end === null ? "" : `New end: ${stamp(u.new_end)}`,
      by: authorName(u.by),
    });
  }
  return {
    id: m.id,
    title: m.title,
    phase: label,
    phaseShort: short,
    when: dayStart(m.starts) === dayStart(m.ends) ? `${weekdayDate(m.starts)}, ${clock(m.starts)} – ${clock(m.ends)} UTC` : `${stamp(m.starts)} – ${stamp(m.ends)}`,
    relative,
    chips: m.parts.map((p) => chip(p, "maintenance")),
    notice: m.notice,
    updates,
    lasted: duration(m.ends - m.starts),
    by: m.by === null ? "" : authorName(m.by),
    announced: stamp(m.announced_at),
  };
}

/** "Nov 6, 09:12 – 10:47 UTC", or with both dates if it crossed midnight. */
export function range(from: Utc, to: Utc): string {
  if (dayStart(from) === dayStart(to)) return `${shortDate(from)}, ${clock(from)} – ${clock(to)} UTC`;
  return `${stamp(from)} – ${stamp(to)}`;
}

// ----------------------------------------------------------------- parts

export interface GroupView {
  name: string;
  /** A group of one part is shown as the part alone. */
  single: boolean;
  status: string;
  statusKey: string;
  parts: PartView[];
}

export interface PartView {
  name: string;
  status: string;
  statusKey: string;
  note: string;
  uptime: string;
  days: DayView[];
}

export interface DayView {
  key: string;
  tip: string;
}

export function groups(board: Board): GroupView[] {
  return GROUPS.map((g) => {
    const w = worst(g.parts.map((p) => board.status(p)));
    return { name: g.name, single: g.parts.length === 1, status: statusLabel(w), statusKey: w, parts: g.parts.map((p) => partView(board, p)) };
  });
}

function partView(board: Board, part: PartId): PartView {
  const status = board.status(part);
  const m = board.maintenanceNow(part);
  return {
    name: partName(part),
    status: statusLabel(status),
    statusKey: status,
    note: m !== undefined && status === "maintenance" ? `until ${stamp(m.ends)}` : "",
    uptime: uptimeText(board.uptime(part)),
    days: board.days(part).map((d) => ({ key: d.worst, tip: dayTip(d) })),
  };
}

/** "Nov 6, 2026 · Partial outage for 1 h 35 min · Delayed email notifications" */
export function dayTip(day: Day): string {
  let what: string;
  if (day.worst === "operational") what = day.events.length === 0 ? "No incidents" : "No impact";
  else what = `${statusLabel(day.worst)} for ${duration(day.seconds[severity(day.worst)])}`;
  let tip = `${date(day.start)} · ${what}`;
  if (day.events.length > 0) tip += " · " + day.events.join("; ");
  return tip;
}

/** "100%", "99.93%": rounded down, so a short outage never shows as 100%. */
export function uptimeText(percent: number): string {
  if (percent >= 100) return "100%";
  const down = Math.floor(percent * 100) / 100;
  return `${Math.min(down, 99.99).toFixed(2)}%`;
}

// ---------------------------------------------------------------- past

export interface MonthView {
  name: string;
  items: PastItem[];
}

export interface PastItem {
  /** The incident's id; empty for maintenance, which has no page. */
  id: string;
  maintenance: boolean;
  title: string;
  when: string;
  lasted: string;
  chips: Chip[];
  summary: string;
  start: Utc;
}

/** Resolved incidents and completed maintenance by month, newest first:
 * always this month and the two before, older months when they have
 * something. */
export function pastMonths(board: Board): MonthView[] {
  const items: PastItem[] = [];
  for (const i of board.resolvedIncidents()) {
    const view = incidentView(i, board.now);
    const last = latest(i);
    items.push({
      id: view.id,
      maintenance: false,
      title: view.title,
      when: view.when,
      lasted: view.lasted,
      chips: view.chips,
      summary: last === undefined ? "" : last.message,
      start: i.opened_at,
    });
  }
  for (const m of board.maintenance("completed")) {
    const view = maintenanceView(m, board.now);
    items.push({
      id: "",
      maintenance: true,
      title: view.title,
      when: range(m.starts, m.ends),
      lasted: view.lasted,
      chips: view.chips,
      summary: view.notice,
      start: m.starts,
    });
  }
  items.sort((a, b) => b.start - a.start);

  const months = new Map<number, MonthView>();
  const entry = (m: Month): MonthView => {
    const key = m.year * 12 + m.month;
    let v = months.get(key);
    if (v === undefined) {
      v = { name: monthName(m), items: [] };
      months.set(key, v);
    }
    return v;
  };
  let m = month(board.now);
  for (let i = 0; i < 3; i++) {
    entry(m);
    m = previousMonth(m);
  }
  for (const item of items) entry(month(item.start)).items.push(item);
  const keys = [...months.keys()].sort((a, b) => b - a);
  return keys.map((k) => months.get(k) as MonthView);
}

// ---------------------------------------------------------------- forms

/** An option of a select. */
export interface Opt {
  value: string;
  label: string;
  selected: boolean;
}

/** The names to pick from, with "Pick your name" first. */
export function people(selected: string): Opt[] {
  const options: Opt[] = [{ value: "", label: "Pick your name", selected: selected === "" }];
  for (const a of AUTHORS) options.push({ value: a, label: authorName(a), selected: a === selected });
  return options;
}

export function stages(allowed: Stage[], selected: string): Opt[] {
  return allowed.map((s) => ({ value: s, label: stageLabel(s), selected: s === selected }));
}

export function nextUpdates(selected: string): Opt[] {
  const choices: [string, string][] = [
    ["", "No time given"],
    ["30", "In 30 minutes"],
    ["60", "In 60 minutes"],
  ];
  return choices.map(([value, label]) => ({ value, label, selected: value === selected }));
}

/** A part's status select. */
export interface PartChoice {
  key: string;
  name: string;
  note: string;
  options: Opt[];
}

/** One select per part; `value` gives each part's chosen status key. */
export function partChoices(value: (part: PartId) => string, note: (part: PartId) => string): PartChoice[] {
  return PARTS.map((part) => {
    const chosen = value(part);
    return {
      key: part,
      name: partFullName(part),
      note: note(part),
      options: STATUS_CHOICES.map((s) => ({ value: s, label: statusLabel(s), selected: s === chosen })),
    };
  });
}

/** A checkbox per part. */
export interface Check {
  key: string;
  name: string;
  checked: boolean;
}

export function partChecks(checked: (part: PartId) => boolean): Check[] {
  return PARTS.map((p) => ({ key: p, name: partFullName(p), checked: checked(p) }));
}

/** The messages next to a form's fields, by field name: the first one per
 * field stays. */
export class Errors {
  private readonly messages = new Map<string, string>();

  add(field: string, error: string): void {
    if (!this.messages.has(field)) this.messages.set(field, error);
  }

  /** The field's message, or "" (the element hides when empty). */
  get(field: string): string {
    return this.messages.get(field) ?? "";
  }
}

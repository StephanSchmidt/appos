// The status module's IO: every kv call, with the keys and the JSON.
// Usecases get these functions as parameters; nothing else touches kv.
// All keys start with "status/".
//
//   status/incidents/<uuid v7>     an incident with all its updates
//   status/changes/<uuid v7>       a part's status changed
//   status/maintenance/<uuid v7>   planned maintenance with its updates
//   status/seeded                  the starting history is stored

import { type Read, getJson, kv, log, newId, scanAllJson, setJson } from "../appos.ts";
import { type Change, type Edit, type Id, type Impact, type Incident, type Maintenance, type MaintenanceUpdate, type Update, AUTHORS, PARTS, STAGES, STATUSES, parseId } from "./domain.ts";
import type { Seed } from "./seed.ts";

const INCIDENTS = "status/incidents/";
const CHANGES = "status/changes/";
const MAINTENANCE = "status/maintenance/";
const SEEDED = "status/seeded";

// Lists read every page of their prefix (scanAllJson): older pages still
// matter, their last change can describe an outage that has not been
// resolved. An entry that doesn't parse is logged and left out; one read by
// its id is Unreadable, which the usecase returns before it writes anything
// (see ERRORS in appos guide).

export function loadIncidents(): Incident[] {
  return readable(scanAllJson<Incident>(INCIDENTS, true), incidentProblem);
}

export function loadIncident(id: Id): Read<Incident> {
  return checked(INCIDENTS + id, getJson<Incident>(INCIDENTS + id), incidentProblem);
}

export function saveIncident(incident: Incident): void {
  setJson(INCIDENTS + incident.id, incidentValue(incident));
}

/** Oldest first, so of two changes in the same second the later one wins. */
export function loadChanges(): Change[] {
  return readable(scanAllJson<Change>(CHANGES, true), changeProblem).reverse();
}

/** Each change under a key of its own: changes at the same time don't
 * write the same key. */
export function addChange(change: Change): void {
  setJson(CHANGES + newId(), changeValue(change));
}

export function loadMaintenance(): Maintenance[] {
  return readable(scanAllJson<Maintenance>(MAINTENANCE, true), maintenanceProblem);
}

export function loadMaintenanceOne(id: Id): Read<Maintenance> {
  return checked(MAINTENANCE + id, getJson<Maintenance>(MAINTENANCE + id), maintenanceProblem);
}

export function saveMaintenance(m: Maintenance): void {
  setJson(MAINTENANCE + m.id, maintenanceValue(m));
}

export function isSeeded(): boolean {
  return kv.get(SEEDED) !== undefined;
}

/** Stores the starting history: 5 writes. */
export function storeSeed(seed: Seed): void {
  saveIncident(seed.incident);
  for (const change of seed.changes) addChange(change);
  saveMaintenance(seed.maintenance);
  kv.set(SEEDED, "1");
}

// JSON.parse checks the JSON against the type, but a string union like
// Status only as a string: "exploded" passes. An entry with an unknown
// name, a uuid that isn't one or a time that isn't whole seconds is left
// out (or Unreadable, when it was read by id).

/** The values of a scan that make sense; the others are logged. */
function readable<T>(entries: [string, T][], problem: (value: T) => string | undefined): T[] {
  const out: T[] = [];
  for (const [key, value] of entries) {
    const p = problem(value);
    if (p === undefined) out.push(value);
    else log.warn(`${key} is unreadable, left out: ${p}`);
  }
  return out;
}

/** A value read by its key: one that doesn't make sense is Unreadable. */
function checked<T>(key: string, read: Read<T>, problem: (value: T) => string | undefined): Read<T> {
  if (!read.ok || read.value === undefined) return read;
  const p = problem(read.value);
  if (p !== undefined) return { ok: false, error: "unreadable", why: `${key} is unreadable: ${p}` };
  return read;
}

const isPart = (x: string): boolean => PARTS.some((p) => p === x);
const isStatus = (x: string): boolean => STATUSES.some((s) => s === x);
const isStage = (x: string): boolean => STAGES.some((s) => s === x);
const isAuthor = (x: string): boolean => AUTHORS.some((a) => a === x);
const isTime = (t: number): boolean => Math.floor(t) === t && Math.abs(t) <= 9007199254740991;
const isMinutes = (n: number): boolean => Math.floor(n) === n && n >= 0 && n <= 4294967295;

function incidentProblem(i: Incident): string | undefined {
  const id = parseId(i.id);
  if (id === undefined) return `id ${i.id} is not a uuid`;
  i.id = id;
  if (!isTime(i.opened_at) || (i.resolved_at !== null && !isTime(i.resolved_at))) return "a time is not in seconds";
  for (const x of i.impact) {
    if (!isPart(x.part) || !isStatus(x.now) || !isStatus(x.worst)) return `unknown part or status in impact ${x.part}`;
  }
  for (const u of i.updates) {
    if (!isStage(u.stage)) return `unknown stage ${u.stage}`;
    if (!isAuthor(u.by)) return `unknown author ${u.by}`;
    if (!isTime(u.at)) return "a time is not in seconds";
    if (u.next_update !== null && !isMinutes(u.next_update)) return `next_update ${u.next_update} is not minutes`;
    for (const e of u.edits) {
      if (!isAuthor(e.by)) return `unknown author ${e.by}`;
      if (!isTime(e.at)) return "a time is not in seconds";
    }
  }
  return undefined;
}

function changeProblem(c: Change): string | undefined {
  if (!isPart(c.part)) return `unknown part ${c.part}`;
  if (!isStatus(c.status)) return `unknown status ${c.status}`;
  if (!isTime(c.at)) return "a time is not in seconds";
  if (c.by !== null && !isAuthor(c.by)) return `unknown author ${c.by}`;
  if (c.incident !== null) {
    const id = parseId(c.incident);
    if (id === undefined) return `incident ${c.incident} is not a uuid`;
    c.incident = id;
  }
  return undefined;
}

function maintenanceProblem(m: Maintenance): string | undefined {
  const id = parseId(m.id);
  if (id === undefined) return `id ${m.id} is not a uuid`;
  m.id = id;
  for (const p of m.parts) if (!isPart(p)) return `unknown part ${p}`;
  if (!isTime(m.starts) || !isTime(m.ends) || !isTime(m.announced_at)) return "a time is not in seconds";
  if (m.by !== null && !isAuthor(m.by)) return `unknown author ${m.by}`;
  for (const u of m.updates) {
    if (!isAuthor(u.by)) return `unknown author ${u.by}`;
    if (!isTime(u.at) || (u.new_end !== null && !isTime(u.new_end))) return "a time is not in seconds";
  }
  return undefined;
}

// The values field by field in serde's order, nothing more: a value read
// from kv keeps fields this app doesn't know when stringified again.

function incidentValue(i: Incident): Incident {
  const value: Incident = {
    id: i.id,
    title: i.title,
    opened_at: i.opened_at,
    resolved_at: i.resolved_at,
    impact: i.impact.map((x): Impact => ({ part: x.part, now: x.now, worst: x.worst })),
    updates: i.updates.map(
      (u): Update => ({
        stage: u.stage,
        message: u.message,
        at: u.at,
        by: u.by,
        next_update: u.next_update,
        edits: u.edits.map((e): Edit => ({ previous: e.previous, at: e.at, by: e.by })),
      }),
    ),
  };
  return value;
}

function changeValue(c: Change): Change {
  const value: Change = { part: c.part, status: c.status, at: c.at, by: c.by, incident: c.incident };
  return value;
}

function maintenanceValue(m: Maintenance): Maintenance {
  const value: Maintenance = {
    id: m.id,
    title: m.title,
    parts: m.parts.slice(),
    starts: m.starts,
    ends: m.ends,
    notice: m.notice,
    by: m.by,
    announced_at: m.announced_at,
    updates: m.updates.map((u): MaintenanceUpdate => ({ message: u.message, at: u.at, by: u.by, new_end: u.new_end })),
  };
  return value;
}

// Usecase: post an update to an open incident, resolving it included.

import type { Read, Unreadable } from "../appos.ts";
import type { Utc } from "../time.ts";
import { storedStatus } from "./board.ts";
import * as db from "./db.ts";
import { type Change, type Id, type Incident, type PartId, type PostUpdate, type Status, affects, isDown, isOpen, post } from "./domain.ts";

/** The incident with the update, or why it wasn't posted. */
export type UpdateResult = { ok: true; incident: Incident } | { ok: false; error: "not-found" | "already-resolved" | "unresolved-parts" } | Unreadable;

export const ALREADY_RESOLVED = "This incident is resolved already. Open a new one if it's back.";
export const UNRESOLVED_PARTS = "A resolved incident can keep a part at “Degraded performance”, not worse. Set the others to Operational.";

/** Stores the incident and the parts whose status changed: at most 10
 * writes. */
export function updateUc(id: Id, cmd: PostUpdate, now: Utc): UpdateResult {
  return updateU(id, cmd, now, db.loadIncident, db.loadChanges, db.saveIncident, db.addChange);
}

export function updateU(
  id: Id,
  cmd: PostUpdate,
  now: Utc,
  loadIncident: (id: Id) => Read<Incident>,
  loadChanges: () => Change[],
  saveIncident: (incident: Incident) => void,
  addChange: (change: Change) => void,
): UpdateResult {
  const read = loadIncident(id);
  if (!read.ok) return read;
  const incident = read.value;
  if (incident === undefined) return { ok: false, error: "not-found" };
  if (!isOpen(incident)) return { ok: false, error: "already-resolved" };
  const history = loadChanges();
  const current = (part: PartId): Status => storedStatus(history, part);
  if (cmd.stage === "resolved") {
    // Every form includes every part: an unchanged outage in another
    // incident must not prevent this one being resolved. Omitted parts
    // keep their current status, as on the desk's other forms.
    const stillDown = incident.impact.some((p) => isDown(cmd.parts.find((chosen) => chosen.part === p.part)?.status ?? current(p.part)));
    const newOutage = cmd.parts.some((p) => isDown(p.status) && !affects(incident, p.part) && p.status !== current(p.part));
    if (stillDown || newOutage) return { ok: false, error: "unresolved-parts" };
  }
  const changed = post(incident, cmd, current, now);
  saveIncident(incident);
  for (const { part, status } of changed) addChange({ part, status, at: now, by: cmd.by, incident: incident.id });
  return { ok: true, incident };
}

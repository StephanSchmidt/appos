// Usecases: schedule maintenance, and post an update to it (e.g. it runs
// late and gets a new end).

import type { Read, Unreadable } from "../appos.ts";
import type { Utc } from "../time.ts";
import * as db from "./db.ts";
import { type Author, type Id, type Maintenance, type NewMaintenance, Fail, intoMaintenance, maintenanceNote, postNote } from "./domain.ts";

export function scheduleUc(cmd: NewMaintenance, now: Utc): Maintenance {
  return scheduleU(cmd, now, db.saveMaintenance);
}

export function scheduleU(cmd: NewMaintenance, now: Utc, save: (m: Maintenance) => void): Maintenance {
  const m = intoMaintenance(cmd, now);
  save(m);
  return m;
}

/** The maintenance with the note, or why it wasn't posted; "refused"
 * says why in words for the desk. */
export type NoteResult =
  | { ok: true; maintenance: Maintenance }
  | { ok: false; error: "not-found" }
  | { ok: false; error: "refused"; message: string }
  | Unreadable;

export function noteUc(id: Id, message: string, newEnd: Utc | null, by: Author, now: Utc): NoteResult {
  return noteU(id, message, newEnd, by, now, db.loadMaintenanceOne, db.saveMaintenance);
}

export function noteU(
  id: Id,
  message: string,
  newEnd: Utc | null,
  by: Author,
  now: Utc,
  load: (id: Id) => Read<Maintenance>,
  save: (m: Maintenance) => void,
): NoteResult {
  const read = load(id);
  if (!read.ok) return read;
  const m = read.value;
  if (m === undefined) return { ok: false, error: "not-found" };
  const note = maintenanceNote(message, newEnd, by, m.starts, now);
  if (note instanceof Fail) return { ok: false, error: "refused", message: note.message };
  postNote(m, note, now);
  save(m);
  return { ok: true, maintenance: m };
}

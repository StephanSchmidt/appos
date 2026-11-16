// Usecase: fix a typo in an update. The update stays, with its old text
// kept for the desk; customers see "edited".

import type { Read, Unreadable } from "../appos.ts";
import type { Utc } from "../time.ts";
import * as db from "./db.ts";
import { type Author, type Id, type Incident, edit } from "./domain.ts";

/** The corrected incident, or why it wasn't changed. */
export type EditResult = { ok: true; incident: Incident } | { ok: false; error: "not-found" | "no-such-update" | "unchanged" } | Unreadable;

export const UNCHANGED = "That's the text it already has.";

export function editUc(id: Id, n: number, text: string, by: Author, now: Utc): EditResult {
  return editU(id, n, text, by, now, db.loadIncident, db.saveIncident);
}

export function editU(
  id: Id,
  n: number,
  text: string,
  by: Author,
  now: Utc,
  loadIncident: (id: Id) => Read<Incident>,
  saveIncident: (incident: Incident) => void,
): EditResult {
  const read = loadIncident(id);
  if (!read.ok) return read;
  const incident = read.value;
  if (incident === undefined) return { ok: false, error: "not-found" };
  const problem = edit(incident, n, text, by, now);
  if (problem !== undefined) return { ok: false, error: problem };
  saveIncident(incident);
  return { ok: true, incident };
}

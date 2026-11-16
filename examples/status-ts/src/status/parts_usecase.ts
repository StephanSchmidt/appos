// Usecase: set parts' status outside an incident, e.g. a part kept at
// Degraded performance when an incident was resolved, now fully fixed.

import type { Utc } from "../time.ts";
import { storedStatus } from "./board.ts";
import * as db from "./db.ts";
import type { Author, Change, PartStatus } from "./domain.ts";

/** Records the parts whose status differs; returns how many: at most 9
 * writes. */
export function partsUc(parts: PartStatus[], by: Author, now: Utc): number {
  return partsU(parts, by, now, db.loadChanges, db.addChange);
}

export function partsU(parts: PartStatus[], by: Author, now: Utc, loadChanges: () => Change[], addChange: (change: Change) => void): number {
  const history = loadChanges();
  const changed = parts.filter((p) => storedStatus(history, p.part) !== p.status);
  for (const { part, status } of changed) addChange({ part, status, at: now, by, incident: null });
  return changed.length;
}

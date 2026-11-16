// Usecase: everything the page shows at this moment. Stores the starting
// history on the very first request.

import type { Utc } from "../time.ts";
import { Board } from "./board.ts";
import * as db from "./db.ts";
import type { Change, Incident, Maintenance } from "./domain.ts";
import { type Seed, seed } from "./seed.ts";

export function boardUc(now: Utc): Board {
  return boardU(now, db.isSeeded, db.storeSeed, db.loadIncidents, db.loadChanges, db.loadMaintenance);
}

export function boardU(
  now: Utc,
  isSeeded: () => boolean,
  storeSeed: (seed: Seed) => void,
  loadIncidents: () => Incident[],
  loadChanges: () => Change[],
  loadMaintenance: () => Maintenance[],
): Board {
  if (!isSeeded()) {
    const s = seed(now);
    storeSeed(s);
    // The request's own writes aren't applied yet; show them anyway.
    return new Board(now, [s.incident], s.changes, [s.maintenance]);
  }
  return new Board(now, loadIncidents(), loadChanges(), loadMaintenance());
}

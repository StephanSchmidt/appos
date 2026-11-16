// Usecase: open an incident.

import type { Utc } from "../time.ts";
import { storedStatus } from "./board.ts";
import * as db from "./db.ts";
import { type Change, type Incident, type OpenIncident, newIncident } from "./domain.ts";

/** Stores the incident and the parts' new statuses: at most 10 writes. */
export function openUc(cmd: OpenIncident, now: Utc): Incident {
  return openU(cmd, now, db.loadChanges, db.saveIncident, db.addChange);
}

export function openU(
  cmd: OpenIncident,
  now: Utc,
  loadChanges: () => Change[],
  saveIncident: (incident: Incident) => void,
  addChange: (change: Change) => void,
): Incident {
  const history = loadChanges();
  const incident = newIncident(cmd, now);
  saveIncident(incident);
  for (const { part, status } of cmd.parts) {
    if (storedStatus(history, part) !== status) addChange({ part, status, at: now, by: cmd.by, incident: incident.id });
  }
  return incident;
}

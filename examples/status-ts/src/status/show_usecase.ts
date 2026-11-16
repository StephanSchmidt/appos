// Usecases: one incident, one maintenance.

import type { Read, Unreadable } from "../appos.ts";
import * as db from "./db.ts";
import type { Id, Incident, Maintenance } from "./domain.ts";

/** The incident, or why it can't be shown. */
export type ShowIncidentResult = { ok: true; incident: Incident } | { ok: false; error: "not-found" } | Unreadable;

/** The maintenance, or why it can't be shown. */
export type ShowMaintenanceResult = { ok: true; maintenance: Maintenance } | { ok: false; error: "not-found" } | Unreadable;

export function showIncidentUc(id: Id): ShowIncidentResult {
  return showIncidentU(id, db.loadIncident);
}

export function showIncidentU(id: Id, loadIncident: (id: Id) => Read<Incident>): ShowIncidentResult {
  const read = loadIncident(id);
  if (!read.ok) return read;
  return read.value === undefined ? { ok: false, error: "not-found" } : { ok: true, incident: read.value };
}

export function showMaintenanceUc(id: Id): ShowMaintenanceResult {
  return showMaintenanceU(id, db.loadMaintenanceOne);
}

export function showMaintenanceU(id: Id, load: (id: Id) => Read<Maintenance>): ShowMaintenanceResult {
  const read = load(id);
  if (!read.ok) return read;
  return read.value === undefined ? { ok: false, error: "not-found" } : { ok: true, maintenance: read.value };
}

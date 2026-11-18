// Usecase: the configure page of a draft.

import type { Booking, CatalogInfo, Draft } from "../../bindings/flow.ts";
import * as flow from "../../bindings/flow.ts";
import type { Read, Unreadable } from "../appos.ts";
import * as db from "./db.ts";

export type ShowResult =
  | { ok: true; draft: Draft; catalog: CatalogInfo; configured: boolean }
  | { ok: false; error: "not-found" | "booked" | "no-package" }
  | Unreadable;

export function showUc(id: string): ShowResult {
  return showU(id, flow.get, flow.booked, flow.catalog, db.loadConfigured);
}

export function showU(
  id: string,
  get: (id: string) => Draft | undefined,
  booked: (id: string) => Booking | undefined,
  catalog: () => CatalogInfo,
  loadConfigured: (id: string) => Read<boolean>,
): ShowResult {
  if (booked(id) !== undefined) return { ok: false, error: "booked" };
  const draft = get(id);
  if (draft === undefined) return { ok: false, error: "not-found" };
  if (draft.plan === undefined) return { ok: false, error: "no-package" };
  const configured = loadConfigured(id);
  if (!configured.ok) return configured;
  return { ok: true, draft, catalog: catalog(), configured: configured.value === true };
}

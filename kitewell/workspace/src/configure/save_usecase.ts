// Usecase: save what the customer chose on the configure page.

import type { Result } from "../../bindings/abi.ts";
import type { Booking, CatalogInfo, Draft } from "../../bindings/flow.ts";
import * as configure from "../../bindings/configure.ts";
import * as flow from "../../bindings/flow.ts";
import * as db from "./db.ts";
import type { Choice } from "./domain.ts";

/** The draft with the choice in it, or why not. `refused` carries the
 * booking functions' reason, in words for the customer. */
export type SaveResult =
  | { ok: true; draft: Draft }
  | { ok: false; error: "not-found" | "booked" }
  | { ok: false; error: "refused"; why: string };

export function saveUc(id: string, choice: Choice): SaveResult {
  return saveU(id, choice, flow.get, flow.booked, flow.catalog, configure.withTemplate, configure.withAddOn, db.markConfigured);
}

export function saveU(
  id: string,
  choice: Choice,
  get: (id: string) => Draft | undefined,
  booked: (id: string) => Booking | undefined,
  catalog: () => CatalogInfo,
  withTemplate: (id: string, template: string | undefined) => Result<Draft, string>,
  withAddOn: (id: string, addOn: string, on: boolean) => Result<Draft, string>,
  markConfigured: (id: string) => void,
): SaveResult {
  if (booked(id) !== undefined) return { ok: false, error: "booked" };
  const draft = get(id);
  if (draft === undefined) return { ok: false, error: "not-found" };
  let saved = withTemplate(id, choice.template);
  for (const addOn of catalog().addOns) {
    if (!saved.ok) break;
    // One that comes with the plan has no switch on the page.
    if (draft.plan !== undefined && addOn.includedIn.includes(draft.plan)) continue;
    saved = withAddOn(id, addOn.id, choice.addOns.includes(addOn.id));
  }
  if (!saved.ok) return { ok: false, error: "refused", why: saved.error };
  markConfigured(id);
  return { ok: true, draft: saved.value };
}

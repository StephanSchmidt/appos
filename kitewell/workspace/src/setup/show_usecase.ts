// Usecase: the set-up page of a booking.

import type { Booking, Draft } from "../../bindings/flow.ts";
import * as flow from "../../bindings/flow.ts";

/** The booking; `not-booked` while the draft is still open, so the
 * customer goes back to review and book. */
export type ShowResult = { ok: true; booking: Booking } | { ok: false; error: "not-found" | "not-booked" };

export function showUc(id: string): ShowResult {
  return showU(id, flow.booked, flow.get);
}

export function showU(id: string, booked: (id: string) => Booking | undefined, get: (id: string) => Draft | undefined): ShowResult {
  const booking = booked(id);
  if (booking !== undefined) return { ok: true, booking };
  return get(id) === undefined ? { ok: false, error: "not-found" } : { ok: false, error: "not-booked" };
}

// The configure module's IO in kv: whether a draft's customer has saved
// this page. Keys start with "configure/".

import { type Read, getJson, setJson } from "../appos.ts";

const SAVED = "configure/saved/";

/** Marks the draft as configured: from now on the page shows what the
 * customer chose, "Start empty" included, and no longer the default. */
export function markConfigured(draft: string): void {
  setJson(SAVED + draft, true);
}

export function loadConfigured(draft: string): Read<boolean> {
  return getJson<boolean>(SAVED + draft);
}

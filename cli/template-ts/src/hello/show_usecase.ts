// Usecase: show one greeting.

import type { Read, Unreadable } from "../appos.ts";
import * as db from "./db.ts";
import type { Greeting } from "./domain.ts";

/** The greeting, or why it can't be shown. Each usecase lists what can go
 * wrong here and nothing else, and the handler handles every case. These
 * are outcomes the data decides; input that doesn't parse never gets this
 * far. */
export type ShowResult = { ok: true; greeting: Greeting } | { ok: false; error: "not-found" } | Unreadable;

export function showUc(id: string): ShowResult {
  return showU(id, db.loadGreeting);
}

export function showU(id: string, loadGreeting: (id: string) => Read<Greeting>): ShowResult {
  const read = loadGreeting(id);
  if (!read.ok) return read;
  return read.value === undefined ? { ok: false, error: "not-found" } : { ok: true, greeting: read.value };
}

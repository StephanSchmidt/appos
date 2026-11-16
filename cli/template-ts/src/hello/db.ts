// The hello module's IO: every kv call, with the keys. Usecases get these
// functions as parameters; nothing else touches kv. All keys start with
// "hello/", so they can't collide with another module's.

import { type Read, getJson, scanJson, setJson } from "../appos.ts";
import type { Greeting } from "./domain.ts";

const HISTORY = "hello/history/";

/** Adds a greeting to the history under a key of its own. Requests that
 * greet at the same time write different keys, so neither has to run again
 * (see STORAGE in appos guide). A count of all hellos in one key would be
 * written by every greeting. */
export function addGreeting(greeting: Greeting): void {
  setJson(key(greeting.id), greeting);
}

/** The greeting with this id, if there is one; Unreadable if what is stored
 * there isn't JSON (see ERRORS in appos guide). */
export function loadGreeting(id: string): Read<Greeting> {
  return getJson<Greeting>(key(id));
}

/** The newest `n` greetings, newest first: a reverse scan starts at the
 * last key. One scan reads them all; one get each would cost more of the
 * request's budget. */
export function loadHistory(n: number): Greeting[] {
  return scanJson<Greeting>(HISTORY, undefined, n, true).map((entry) => entry[1]);
}

/** Ids sort by time, so the key order (what a scan returns) is the order
 * the greetings came in. */
export function key(id: string): string {
  return HISTORY + id;
}

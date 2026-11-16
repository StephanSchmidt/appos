// Usecase: list the newest greetings.

import * as db from "./db.ts";
import { type Greeting, HISTORY } from "./domain.ts";

/** The newest greetings, newest first. */
export function listUc(): Greeting[] {
  return listU(db.loadHistory);
}

export function listU(loadHistory: (n: number) => Greeting[]): Greeting[] {
  return loadHistory(HISTORY);
}

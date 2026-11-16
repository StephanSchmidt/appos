// Usecase: greet someone.

import { log } from "../appos.ts";
import * as db from "./db.ts";
import { type Greeting, type Name, newGreeting } from "./domain.ts";

/** Greets `name` at `now`, adds the greeting to the history and logs it.
 * It takes a `Name`, not text: the handler has checked the input already. */
export function greetUc(name: Name, now: string): Greeting {
  return greetU(name, now, db.addGreeting, log.info);
}

export function greetU(name: Name, now: string, addGreeting: (greeting: Greeting) => void, log: (line: string) => void): Greeting {
  const greeting = newGreeting(name, now);
  addGreeting(greeting);
  // JSON quotes the name, so a name with a line break can't fake a log line.
  log(`hello: ${JSON.stringify(greeting.name)}`);
  return greeting;
}

// The hello module's data and its rules: plain TypeScript, no IO.

import { newId } from "../appos.ts";

/** How many of the newest greetings the page shows. */
export const HISTORY = 10;

/** The longest name, in characters. The form says the same (maxlength), so
 * the browser stops typing there; the server checks it anyway. */
export const NAME_MAX = 40;

/** Why text is not a `Name`. Code and tests look at `kind`;
 * `nameErrorText` is the message for the visitor. */
export type NameError = { kind: "too-long"; max: number };

export function nameErrorText(error: NameError): string {
  switch (error.kind) {
    case "too-long":
      return `That's a long name! Please keep it to ${error.max} characters.`;
  }
}

/** A name to greet. Parse, don't validate: the only way to make one is
 * `Name.parse`, so code that takes a `Name` never sees an invalid one.
 * Input is checked once, where it comes in, and the rest of the app relies
 * on the type. */
export class Name {
  readonly text: string;

  private constructor(text: string) {
    this.text = text;
  }

  /** Parses what a visitor typed: trimmed, at most NAME_MAX characters;
   * empty means the world. A form with more fields parses each field like
   * this, into its own type. */
  static parse(input: string): { ok: true; name: Name } | { ok: false; error: NameError } {
    const name = input.trim();
    if ([...name].length > NAME_MAX) return { ok: false, error: { kind: "too-long", max: NAME_MAX } };
    return { ok: true, name: new Name(name === "" ? "world" : name) };
  }
}

/** One hello, as stored in kv (JSON). */
export interface Greeting {
  /** From `newId`: ids sort in the order the greetings came in. */
  id: string;
  name: string;
  /** When, on the platform's clock, as RFC 3339 text:
   * "2026-11-11T14:05:09+01:00". */
  at: string;
}

/** A new greeting, with a new id. */
export function newGreeting(name: Name, at: string): Greeting {
  return { id: newId(), name: name.text, at };
}

/** The date, "2026-11-11". */
export function date(greeting: Greeting): string {
  return greeting.at.slice(0, 10);
}

/** The time of day, "14:05". */
export function time(greeting: Greeting): string {
  return greeting.at.slice(11, 16);
}

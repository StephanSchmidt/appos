import assert from "node:assert/strict";
import { test } from "node:test";
import { NAME_MAX, Name, date, nameErrorText, newGreeting, time } from "./domain.ts";

const NOW = "2026-11-11T14:05:09+01:00";

function name(input: string): string {
  const parsed = Name.parse(input);
  assert.ok(parsed.ok);
  return parsed.name.text;
}

test("names are trimmed", () => {
  assert.equal(name("  Ann "), "Ann");
});

test("no name greets the world", () => {
  assert.equal(name("   "), "world");
});

test("long names are refused", () => {
  const max = "é".repeat(NAME_MAX);
  assert.equal(name(max), max);
  const parsed = Name.parse(max + "x");
  assert.ok(!parsed.ok);
  assert.deepEqual(parsed.error, { kind: "too-long", max: NAME_MAX });
  assert.match(nameErrorText(parsed.error), /40 characters/);
});

test("a greeting knows its date and time", () => {
  const parsed = Name.parse("Ann");
  assert.ok(parsed.ok);
  const greeting = newGreeting(parsed.name, NOW);
  assert.equal(greeting.name, "Ann");
  assert.equal(date(greeting), "2026-11-11");
  assert.equal(time(greeting), "14:05");
});

test("greetings made one after the other sort in that order", () => {
  const parsed = Name.parse("Ann");
  assert.ok(parsed.ok);
  const first = newGreeting(parsed.name, NOW);
  assert.ok(first.id < newGreeting(parsed.name, NOW).id);
});

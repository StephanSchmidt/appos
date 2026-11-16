import assert from "node:assert/strict";
import { test } from "node:test";
import { type Greeting, Name, newGreeting } from "./domain.ts";
import * as view from "./view.ts";

function ann(): Greeting {
  const parsed = Name.parse("Ann");
  assert.ok(parsed.ok);
  return newGreeting(parsed.name, "2026-11-11T14:05:09+01:00");
}

function index(history: Greeting[], name = ""): view.Index {
  return { base: "/hello", history, name, error: undefined };
}

test("page before any hello", () => {
  const page = view.index(index([])).text;
  assert.match(page, /Hello!/);
  assert.match(page, /Nobody yet/);
  assert.match(page, /action="\/hello\/greet"/);
  assert.match(page, /maxlength="40"/);
});

test("greeting and history show the newest hello", () => {
  const greeting = ann();
  const t = index([greeting]);
  assert.match(view.greeting(t).text, /Hello, Ann!/);
  const history = view.history(t).text;
  assert.match(history, /14:05/);
  assert.ok(history.includes(`href="/hello/greetings/${greeting.id}"`), history);
});

test("an error shows next to what was typed", () => {
  const t: view.Index = { ...index([], "Annnn"), error: { kind: "too-long", max: 40 } };
  const page = view.index(t).text;
  assert.match(page, /value="Annnn"/);
  assert.match(page, /keep it to 40 characters/);
});

test("no error renders nothing", () => {
  assert.equal(view.nameError(index([])).text, "");
});

test("a greeting's page shows when", () => {
  const page = view.greetingPage({ base: "/hello", greeting: ann() }).text;
  assert.match(page, /Hello, Ann!/);
  assert.match(page, /2026-11-11 at 14:05/);
});

test("names are escaped", () => {
  const parsed = Name.parse("<b>Ann</b>");
  assert.ok(parsed.ok);
  const evil = newGreeting(parsed.name, "2026-11-11T14:05:09+01:00");
  const page = view.index(index([evil], "<b>")).text;
  assert.ok(!page.includes("<b>"), page);
  assert.match(page, /&#60;b&#62;Ann/);
});

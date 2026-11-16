import assert from "node:assert/strict";
import { test } from "node:test";
import { at } from "../time.ts";
import type { Incident } from "./domain.ts";
import { editU } from "./edit_usecase.ts";
import { seed } from "./seed.ts";

const NOW = at(2026, 11, 13, 14, 5);

test("fixes the typo and keeps the old text", () => {
  const incident = seed(NOW).incident;
  const saved: Incident[] = [];
  const result = editU(incident.id, 0, "Fixed text.", "jonas", NOW, () => ({ ok: true, value: incident }), (i) => saved.push(i));
  assert.ok(result.ok);
  assert.equal(result.incident.updates[0].message, "Fixed text.");
  assert.equal(result.incident.updates[0].edits.length, 1);
  assert.equal(saved.length, 1);
});

test("the same text is unchanged and saves nothing", () => {
  const incident = seed(NOW).incident;
  const saved: Incident[] = [];
  const text = incident.updates[0].message;
  const result = editU(incident.id, 0, text, "jonas", NOW, () => ({ ok: true, value: incident }), (i) => saved.push(i));
  assert.deepEqual(result, { ok: false, error: "unchanged" });
  assert.equal(saved.length, 0);
});

test("an unknown incident is not found", () => {
  const result = editU("an-id", 0, "Fixed text.", "jonas", NOW, () => ({ ok: true, value: undefined }), () => assert.fail("saved"));
  assert.deepEqual(result, { ok: false, error: "not-found" });
});

test("an incident that can't be read is an error and nothing is saved", () => {
  const unreadable = { ok: false, error: "unreadable", why: "status/incidents/1 is unreadable: unknown author bob" } as const;
  const saved: Incident[] = [];
  const result = editU("an-id", 0, "Fixed text.", "jonas", NOW, () => unreadable, (i) => saved.push(i));
  assert.deepEqual(result, unreadable);
  assert.equal(saved.length, 0);
});

import assert from "node:assert/strict";
import { test } from "node:test";
import { HOUR, at } from "../time.ts";
import type { Maintenance } from "./domain.ts";
import { noteU } from "./maintenance_usecase.ts";
import { seed } from "./seed.ts";

const NOW = at(2026, 11, 13, 14, 5);

test("a note with a new end moves the window", () => {
  const m = seed(NOW).maintenance;
  const saved: Maintenance[] = [];
  const result = noteU(m.id, "Runs late.", m.ends + HOUR, "mei", NOW, () => ({ ok: true, value: m }), (x) => saved.push(x));
  assert.ok(result.ok);
  assert.equal(result.maintenance.ends, at(2026, 11, 22, 4, 0));
  assert.equal(result.maintenance.updates.length, 1);
  assert.equal(saved.length, 1);
});

test("a new end in the past is refused and nothing is saved", () => {
  const m = seed(NOW).maintenance;
  const saved: Maintenance[] = [];
  const result = noteU(m.id, "Runs late.", NOW - HOUR, "mei", NOW, () => ({ ok: true, value: m }), (x) => saved.push(x));
  assert.deepEqual(result, { ok: false, error: "refused", message: "The new end must be in the future." });
  assert.equal(saved.length, 0);
});

test("an unknown maintenance is not found", () => {
  const result = noteU("an-id", "Runs late.", null, "mei", NOW, () => ({ ok: true, value: undefined }), () => assert.fail("saved"));
  assert.deepEqual(result, { ok: false, error: "not-found" });
});

test("a maintenance that can't be read is an error and nothing is saved", () => {
  const unreadable = { ok: false, error: "unreadable", why: "status/maintenance/1 is unreadable: a time is not in seconds" } as const;
  const saved: Maintenance[] = [];
  const result = noteU("an-id", "Runs late.", null, "mei", NOW, () => unreadable, (x) => saved.push(x));
  assert.deepEqual(result, unreadable);
  assert.equal(saved.length, 0);
});

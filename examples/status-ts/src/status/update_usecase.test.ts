import assert from "node:assert/strict";
import { test } from "node:test";
import type { Read } from "../appos.ts";
import { HOUR, at } from "../time.ts";
import { type Change, type Incident, type PostUpdate, newIncident, postUpdate } from "./domain.ts";
import { seed } from "./seed.ts";
import { type UpdateResult, updateU } from "./update_usecase.ts";

const NOW = at(2026, 11, 13, 14, 5);

/** An incident opened an hour ago: the API is degraded. */
function open(): Incident {
  const parts = [{ part: "api" as const, status: "degraded" as const }];
  return newIncident({ title: "Slow API", stage: "investigating", message: "We're looking into it.", nextUpdate: 30, by: "amara", parts }, NOW - HOUR);
}

const RESOLVE: PostUpdate = { stage: "resolved", message: "Fixed.", nextUpdate: null, by: "jonas", parts: [{ part: "api", status: "operational" }] };

/** The writes an update makes. */
interface Writes {
  incidents: Incident[];
  changes: Change[];
}

function update(id: string, load: () => Read<Incident>, cmd: PostUpdate, history: Change[]): [UpdateResult, Writes] {
  const writes: Writes = { incidents: [], changes: [] };
  const result = updateU(id, cmd, NOW, load, () => history, (i) => writes.incidents.push(i), (c) => writes.changes.push(c));
  return [result, writes];
}

test("resolving saves the incident and the parts that changed", () => {
  const incident = open();
  const history: Change[] = [{ part: "api", status: "degraded", at: NOW - HOUR, by: "amara", incident: incident.id }];
  const [result, writes] = update(incident.id, () => ({ ok: true, value: incident }), RESOLVE, history);
  assert.ok(result.ok);
  assert.equal(result.incident.resolved_at, NOW);
  assert.equal(writes.incidents.length, 1);
  assert.deepEqual(writes.changes, [{ part: "api", status: "operational", at: NOW, by: "jonas", incident: incident.id }]);
});

test("an update refreshes incident impact even when a part was already restored elsewhere", () => {
  for (const parts of [RESOLVE.parts, []]) {
    const incident = open();
    const history: Change[] = [{ part: "api", status: "operational", at: NOW, by: "mei", incident: null }];
    const [result, writes] = update(incident.id, () => ({ ok: true, value: incident }), { ...RESOLVE, stage: "monitoring", parts }, history);
    assert.ok(result.ok);
    assert.deepEqual(result.incident.impact, [{ part: "api", now: "operational", worst: "degraded" }]);
    assert.deepEqual(writes.changes, []);
  }
});

test("unchanged outages belonging to another incident do not become this incident's impact", () => {
  const incident = open();
  const history: Change[] = [{ part: "email", status: "major", at: NOW, by: "mei", incident: null }];
  const cmd = postUpdate("resolved", "Fixed.", 30, "jonas", [...RESOLVE.parts, { part: "email", status: "major" }]);
  const [result, writes] = update(incident.id, () => ({ ok: true, value: incident }), cmd, history);
  assert.ok(result.ok);
  assert.equal(result.incident.updates.at(-1)?.next_update, null);
  assert.deepEqual(result.incident.impact, [{ part: "api", now: "operational", worst: "degraded" }]);
  assert.deepEqual(writes.changes, []);
});

test("an incident cannot resolve while its own part is still down, even if omitted from the form", () => {
  for (const parts of [[], [{ part: "api" as const, status: "partial" as const }]]) {
    const incident = open();
    const history: Change[] = [{ part: "api", status: "partial", at: NOW, by: "amara", incident: incident.id }];
    const [result, writes] = update(incident.id, () => ({ ok: true, value: incident }), { ...RESOLVE, parts }, history);
    assert.deepEqual(result, { ok: false, error: "unresolved-parts" });
    assert.equal(incident.resolved_at, null);
    assert.equal(incident.updates.length, 1);
    assert.deepEqual(writes, { incidents: [], changes: [] });
  }
});

test("resolving cannot introduce a new outage", () => {
  const incident = open();
  const cmd: PostUpdate = { ...RESOLVE, parts: [...RESOLVE.parts, { part: "email", status: "major" }] };
  const [result, writes] = update(incident.id, () => ({ ok: true, value: incident }), cmd, []);
  assert.deepEqual(result, { ok: false, error: "unresolved-parts" });
  assert.equal(incident.resolved_at, null);
  assert.deepEqual(writes, { incidents: [], changes: [] });
});

test("a resolved incident takes no more updates", () => {
  const incident = seed(NOW).incident;
  const [result, writes] = update(incident.id, () => ({ ok: true, value: incident }), RESOLVE, []);
  assert.deepEqual(result, { ok: false, error: "already-resolved" });
  assert.deepEqual(writes, { incidents: [], changes: [] });
});

test("an unknown incident is not found", () => {
  const [result, writes] = update("an-id", () => ({ ok: true, value: undefined }), RESOLVE, []);
  assert.deepEqual(result, { ok: false, error: "not-found" });
  assert.deepEqual(writes, { incidents: [], changes: [] });
});

test("an incident that can't be read is an error and nothing is written", () => {
  const unreadable = { ok: false, error: "unreadable", why: "status/incidents/1 is unreadable: id 1 is not a uuid" } as const;
  const [result, writes] = update("an-id", () => unreadable, RESOLVE, []);
  assert.deepEqual(result, unreadable);
  assert.deepEqual(writes, { incidents: [], changes: [] });
});

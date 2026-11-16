import assert from "node:assert/strict";
import { test } from "node:test";
import { at } from "../time.ts";
import { seed } from "./seed.ts";
import { showIncidentU, showMaintenanceU } from "./show_usecase.ts";

const NOW = at(2026, 11, 13, 14, 5);

test("shows the incident", () => {
  const incident = seed(NOW).incident;
  const shown = showIncidentU(incident.id, () => ({ ok: true, value: incident }));
  assert.ok(shown.ok);
  assert.equal(shown.incident.title, "Delayed email notifications");
});

test("an unknown incident is not found", () => {
  assert.deepEqual(
    showIncidentU("an-id", () => ({ ok: true, value: undefined })),
    { ok: false, error: "not-found" },
  );
});

test("an incident that can't be read is an error", () => {
  const unreadable = { ok: false, error: "unreadable", why: "status/incidents/1 is unreadable: unknown stage exploded" } as const;
  assert.deepEqual(showIncidentU("an-id", () => unreadable), unreadable);
});

test("shows the maintenance", () => {
  const m = seed(NOW).maintenance;
  const shown = showMaintenanceU(m.id, () => ({ ok: true, value: m }));
  assert.ok(shown.ok);
  assert.equal(shown.maintenance.id, m.id);
});

test("an unknown maintenance is not found", () => {
  assert.deepEqual(
    showMaintenanceU("an-id", () => ({ ok: true, value: undefined })),
    { ok: false, error: "not-found" },
  );
});

test("a maintenance that can't be read is an error", () => {
  const unreadable = { ok: false, error: "unreadable", why: "status/maintenance/1 is unreadable: unknown part toaster" } as const;
  assert.deepEqual(showMaintenanceU("an-id", () => unreadable), unreadable);
});

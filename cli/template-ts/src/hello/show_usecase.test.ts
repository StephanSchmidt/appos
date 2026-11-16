import assert from "node:assert/strict";
import { test } from "node:test";
import { showU } from "./show_usecase.ts";

test("shows the greeting", () => {
  const shown = showU("an-id", (id) => ({ ok: true, value: { id, name: "Ann", at: "2026-11-11T14:05:09+01:00" } }));
  assert.ok(shown.ok);
  assert.equal(shown.greeting.name, "Ann");
});

test("an unknown id is not found", () => {
  assert.deepEqual(
    showU("an-id", () => ({ ok: true, value: undefined })),
    { ok: false, error: "not-found" },
  );
});

test("a greeting that can't be read is an error", () => {
  const unreadable = { ok: false, error: "unreadable", why: "hello/history/1 is unreadable: Unexpected token" } as const;
  assert.deepEqual(showU("an-id", () => unreadable), unreadable);
});

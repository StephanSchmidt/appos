import assert from "node:assert/strict";
import { test } from "node:test";
import { ID, booking, draft } from "../testing.ts";
import { addU } from "./add_usecase.ts";
import { showU } from "./show_usecase.ts";

test("a booking, a draft still open, or nothing", () => {
  assert.ok(showU(ID, () => booking(), () => undefined).ok);
  assert.deepEqual(showU(ID, () => undefined, () => draft()), { ok: false, error: "not-booked" });
  assert.deepEqual(showU(ID, () => undefined, () => undefined), { ok: false, error: "not-found" });
});

test("adds with what was typed, trimmed", () => {
  const added = addU(ID, " Tom ", " tom@nordwind.example ", (_, name, email) => ({ ok: true, value: { name, email, invite: "link" } }));
  assert.deepEqual(added, { ok: true, value: { name: "Tom", email: "tom@nordwind.example", invite: "link" } });
});

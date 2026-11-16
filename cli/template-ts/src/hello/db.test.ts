import assert from "node:assert/strict";
import { test } from "node:test";
import { newId } from "../appos.ts";
import { key } from "./db.ts";

test("keys are the prefix and the id", () => {
  const id = newId();
  assert.equal(key(id), `hello/history/${id}`);
});

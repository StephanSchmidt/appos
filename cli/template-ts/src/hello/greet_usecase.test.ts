import assert from "node:assert/strict";
import { test } from "node:test";
import { Name } from "./domain.ts";
import { greetU } from "./greet_usecase.ts";

test("greets, adds and logs", () => {
  const added: string[] = [];
  const logged: string[] = [];
  const parsed = Name.parse("Ann");
  assert.ok(parsed.ok);

  const greeting = greetU(
    parsed.name,
    "2026-11-11T14:05:09+01:00",
    (g) => added.push(g.name),
    (line) => logged.push(line),
  );

  assert.equal(greeting.name, "Ann");
  assert.deepEqual(added, ["Ann"]);
  assert.deepEqual(logged, ['hello: "Ann"']);
});

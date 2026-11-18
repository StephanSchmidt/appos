import assert from "node:assert/strict";
import { test } from "node:test";
import type { Result } from "../../bindings/abi.ts";
import type { Booking, Draft } from "../../bindings/flow.ts";
import { CATALOG, ID, booking, draft } from "../testing.ts";
import { saveU } from "./save_usecase.ts";

function fakes(open: Draft | undefined, done: Booking | undefined, refuse = "") {
  const calls: string[] = [];
  const answer = (): Result<Draft, string> => (refuse === "" ? { ok: true, value: draft() } : { ok: false, error: refuse });
  return {
    calls,
    run: (template: string | undefined, addOns: string[]) =>
      saveU(
        ID,
        { template, addOns },
        () => open,
        () => done,
        () => CATALOG,
        (_, t) => {
          calls.push(`template ${t}`);
          return answer();
        },
        (_, a, on) => {
          calls.push(`${a} ${on}`);
          return answer();
        },
        () => calls.push("configured"),
      ),
  };
}

test("saves the template and each switch, then remembers the page was saved", () => {
  const f = fakes(draft(), undefined);
  assert.ok(f.run("agency-capacity", []).ok);
  assert.deepEqual(f.calls, ["template agency-capacity", "ai-assistant false", "configured"]);
});

test("an add-on that comes with the plan isn't switched", () => {
  const f = fakes(draft({ plan: "business" }), undefined);
  f.run(undefined, []);
  assert.deepEqual(f.calls, ["template undefined", "configured"]);
});

test("the booking functions' reason, as it is", () => {
  const f = fakes(draft(), undefined, "There is no template nope");
  assert.deepEqual(f.run("nope", []), { ok: false, error: "refused", why: "There is no template nope" });
  assert.deepEqual(f.calls, ["template nope"], "stops at the first no, and doesn't remember");
});

test("a booked or unknown draft changes nothing", () => {
  assert.deepEqual(fakes(draft(), booking()).run(undefined, []), { ok: false, error: "booked" });
  assert.deepEqual(fakes(undefined, undefined).run(undefined, []), { ok: false, error: "not-found" });
});

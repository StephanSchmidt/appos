import assert from "node:assert/strict";
import { test } from "node:test";
import { CATALOG, draft } from "../testing.ts";
import { addOns, defaultTemplate, parseChoice, shownTemplate } from "./domain.ts";

test("the Agency package starts with the agency template, every other package with a quarter", () => {
  assert.equal(defaultTemplate(draft({ package: "Agency package", plan: "business" })), "agency-capacity");
  assert.equal(defaultTemplate(draft()), "plan-a-quarter");
});

test("the default shows until the customer has saved the page", () => {
  assert.equal(shownTemplate(draft(), false), "plan-a-quarter");
  assert.equal(shownTemplate(draft(), true), "", "they chose Start empty");
  assert.equal(shownTemplate(draft({ template: "agency-capacity" }), false), "agency-capacity");
});

test("the assistant has a price, or comes with Business", () => {
  assert.deepEqual(addOns(CATALOG, draft({ addOns: ["ai-assistant"] })), [
    { id: "ai-assistant", name: "AI planning assistant", price: "$4 per user per month", included: false, on: true },
  ]);
  const [business] = addOns(CATALOG, draft({ plan: "business" }));
  assert.ok(business.included && business.on);
});

test("the form's choice", () => {
  assert.deepEqual(
    parseChoice([
      ["draft", "x"],
      ["add-on", "ai-assistant"],
      ["template", "agency-capacity"],
    ]),
    { template: "agency-capacity", addOns: ["ai-assistant"] },
  );
  assert.deepEqual(parseChoice([["template", ""]]), { template: undefined, addOns: [] });
});

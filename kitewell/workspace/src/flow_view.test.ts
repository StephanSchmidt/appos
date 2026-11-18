import assert from "node:assert/strict";
import { test } from "node:test";
import { isDraftId, money, quote, steps } from "./flow_view.ts";
import { ID, draft } from "./testing.ts";

test("money in dollars", () => {
  assert.equal(money(725000), "$7,250");
  assert.equal(money(516250), "$5,162.50");
  assert.equal(money(516205), "$5,162.05");
  assert.equal(money(-1274400), "-$12,744");
  assert.equal(money(11611200), "$116,112");
  assert.equal(money(400), "$4");
});

test("a draft id is 32 hex digits", () => {
  assert.ok(isDraftId(ID));
  assert.ok(!isDraftId(undefined));
  assert.ok(!isDraftId(ID.toUpperCase()));
  assert.ok(!isDraftId(`${ID}0`));
});

test("the quote says what each invoice is", () => {
  const text = quote(draft({ billing: "quarterly", years: 2, paymentTerms: "net45" })).text;
  assert.match(text, /<td>Team, 12 users, yearly<\/td><td>\$3,480<\/td>/);
  assert.match(text, /One invoice a quarter, \$3,480, payable in 45 days\. Over the 2 years: \$3,480\./);
  assert.doesNotMatch(text, /problems/);
  assert.match(quote(draft({ quote: { ...draft().quote, problems: ["Choose a package"] } })).text, /<li>Choose a package<\/li>/);
});

test("the current step is marked", () => {
  assert.match(steps(2).text, /<li class="done"><span>Package<\/span><\/li><li class="now" aria-current="step"><span>Workspace<\/span><\/li><li><span>Terms/);
});

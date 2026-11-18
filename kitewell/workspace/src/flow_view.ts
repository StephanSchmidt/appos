// What both pages of the app share: the steps of the booking flow, the
// quote as the customer sees it, the page around them, and the page for a
// booking that is gone. The prices come from the booking functions; this
// file only writes them in dollars.

import type { Billing, Draft, PaymentTerms } from "../bindings/flow.ts";
import { type Html, empty, html, page, style, title } from "./appos.ts";
import { STYLE } from "./style.ts";

/** Where every booking starts, on the platform. */
export const START = "/book";

/** The steps of the booking flow, `now` being 1 to 5. */
export function steps(now: number): Html {
  const names = ["Package", "Workspace", "Terms", "Review", "Set up"];
  return html`<ol class="steps" aria-label="Booking steps">${names.map((name, i) => {
    if (i + 1 === now) return html`<li class="now" aria-current="step"><span>${name}</span></li>`;
    return i + 1 < now ? html`<li class="done"><span>${name}</span></li>` : html`<li><span>${name}</span></li>`;
  })}</ol>`;
}

/** The draft's quote: each line a year, the total, how the invoices go out. */
export function quote(draft: Draft): Html {
  const q = draft.quote;
  const contract = draft.years > 1 ? ` Over the ${draft.years} years: ${money(q.contractCents)}.` : "";
  return html`<h2>Your price</h2>
<table>
  ${q.lines.map((line) => html`<tr><td>${line.label}</td><td>${money(line.cents)}</td></tr>`)}
  <tr class="total"><td>Per year</td><td>${money(q.perYearCents)}</td></tr>
</table>
<p class="note">One invoice ${every(draft.billing)}, ${money(q.perInvoiceCents)}, payable in ${days(draft.paymentTerms)} days.${contract} Prices without VAT.</p>
${q.problems.length === 0 ? empty : html`<ul class="problems">${q.problems.map((p) => html`<li>${p}</li>`)}</ul>`}`;
}

function every(billing: Billing): string {
  switch (billing) {
    case "monthly":
      return "a month";
    case "quarterly":
      return "a quarter";
    case "yearly":
      return "a year";
  }
}

export function days(terms: PaymentTerms): number {
  switch (terms) {
    case "net30":
      return 30;
    case "net45":
      return 45;
    case "net60":
      return 60;
  }
}

/** 725000 as "$7,250", 516250 as "$5,162.50", -1274400 as "-$12,744". */
export function money(cents: number): string {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  const dollars = String(Math.floor(abs / 100));
  let grouped = "";
  for (let i = 0; i < dollars.length; i++) {
    if (i > 0 && (dollars.length - i) % 3 === 0) grouped += ",";
    grouped += dollars[i];
  }
  const rest = abs % 100;
  if (rest === 0) return `${sign}$${grouped}`;
  return `${sign}$${grouped}.${rest < 10 ? "0" : ""}${rest}`;
}

/** A draft's id is 32 hex digits; other text can't be one. */
export function isDraftId(text: string | undefined): text is string {
  if (text === undefined || text.length !== 32) return false;
  for (const c of text) {
    if (!((c >= "0" && c <= "9") || (c >= "a" && c <= "f"))) return false;
  }
  return true;
}

/** The whole page, in Kitewell's look. */
export function fullPage(heading: string, body: Html): void {
  style(STYLE);
  title(heading);
  page.setHtml("page", body.text);
}

/** A booking that is unknown, or no longer open here: 404 and a way back
 * to the start. */
export function gone(): void {
  page.setStatus(404);
  fullPage(
    "Start again",
    html`<div class="flow">
  <section class="card talk">
    <h1>This booking isn't open any more</h1>
    <p class="lead">It was booked already, or the link is incomplete.</p>
    <a class="button" href="${START}">Start again</a>
  </section>
</div>`,
  );
}

// The configure module's data and rules: plain TypeScript, no IO. What the
// page offers comes from the catalog; which template is preselected is ours.

import type { CatalogInfo, Draft } from "../../bindings/flow.ts";
import { money } from "../flow_view.ts";

/** Sales' package for agencies (packages app), by its name on the draft. */
export const AGENCY_PACKAGE = "Agency package";

/** The template most people keep: Agency capacity plan for the Agency
 * package, Plan a quarter for everything else. */
export function defaultTemplate(draft: Draft): string {
  return draft.package === AGENCY_PACKAGE ? "agency-capacity" : "plan-a-quarter";
}

/** The template the page shows as chosen, "" for "Start empty". The draft's
 * own once the customer has saved this page; before that the default, which
 * the draft doesn't have yet. */
export function shownTemplate(draft: Draft, configured: boolean): string {
  if (draft.template !== undefined) return draft.template;
  return configured ? "" : defaultTemplate(draft);
}

/** An add-on as the page shows it. */
export interface AddOn {
  id: string;
  name: string;
  /** "$4 per user per month". */
  price: string;
  /** Comes with the plan: no switch. */
  included: boolean;
  on: boolean;
}

export function addOns(catalog: CatalogInfo, draft: Draft): AddOn[] {
  return catalog.addOns.map((a) => {
    const included = draft.plan !== undefined && a.includedIn.includes(draft.plan);
    return {
      id: a.id,
      name: a.name,
      price: `${money(a.monthlyCents)} per user per month`,
      included,
      on: included || draft.addOns.includes(a.id),
    };
  });
}

/** What the customer chose on the page. */
export interface Choice {
  /** undefined: start empty. */
  template: string | undefined;
  /** The add-ons switched on. */
  addOns: string[];
}

/** The choice from the form: one `template`, an `add-on` per switch that is
 * on. Whether a template or add-on exists is the booking functions' call. */
export function parseChoice(form: [string, string][]): Choice {
  let template: string | undefined;
  const on: string[] = [];
  for (const [name, value] of form) {
    if (name === "template") template = value === "" ? undefined : value;
    if (name === "add-on") on.push(value);
  }
  return { template, addOns: on };
}

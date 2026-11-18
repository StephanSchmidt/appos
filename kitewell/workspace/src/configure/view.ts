// The configure page. The price and the error are parts of their own: a
// switch or template the customer changes saves at once (htmx), and only
// these two parts come back.

import type { CatalogInfo, Draft } from "../../bindings/flow.ts";
import { type Html, empty, html } from "../appos.ts";
import { quote, steps } from "../flow_view.ts";
import type { AddOn } from "./domain.ts";

export interface Page {
  base: string;
  draft: Draft;
  addOns: AddOn[];
  catalog: CatalogInfo;
  /** The template shown as chosen, "" for "Start empty". */
  template: string;
  error: string;
  back: string;
}

export function page(t: Page): Html {
  const action = `${t.base}/configure`;
  return html`<div class="flow">
  ${steps(2)}

  <header class="intro">
    <h1>Your workspace</h1>
    <p class="lead">${t.draft.package} for ${t.draft.users} people. Choose what your workspace starts with.</p>
  </header>

  <div class="split">
    <form class="main" method="post" action="${action}" hx-post="${action}" hx-trigger="change" hx-swap="none">
      <input type="hidden" name="draft" value="${t.draft.id}">
      <section class="card">
        <h2>Add-ons</h2>
        ${t.addOns.map(addOn)}
      </section>
      <section class="card">
        <h2>Your first project</h2>
        <p class="note">We set it up from a template, so your team has a plan to start from on day one.</p>
        <div class="templates">
          ${t.catalog.templates.map((tpl) => template(tpl.id, tpl.name, tpl.description, t.template))}
          ${template("", "Start empty", "No project yet. You start from a blank workspace.", t.template)}
        </div>
      </section>
      <p class="error" id="configure_error" aria-live="polite">${t.error}</p>
      <div class="actions"><button>Continue</button><a class="back" href="${t.back}">Back</a></div>
    </form>

    <aside class="card quote summary" id="quote">${quote(t.draft)}</aside>
  </div>
</div>`;
}

function addOn(a: AddOn): Html {
  if (a.included) {
    return html`<div class="switch"><span class="included" role="img" aria-label="Included"></span><span><strong>${a.name}</strong><small>Included in your package</small></span></div>`;
  }
  return html`<label class="switch"><input type="checkbox" role="switch" name="add-on" value="${a.id}"${a.on ? html` checked` : empty}><span><strong>${a.name}</strong><small>${a.price}</small></span></label>`;
}

function template(id: string, name: string, description: string, chosen: string): Html {
  return html`<label class="template"><input type="radio" name="template" value="${id}"${id === chosen ? html` checked` : empty}><strong>${name}</strong><small>${description}</small></label>`;
}

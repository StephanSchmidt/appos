// The configure module's handlers. Where the customer goes next, and back,
// the booking functions say (flow.next, flow.link): the next step may be
// another team's app or the platform's own page.

import * as flow from "../../bindings/flow.ts";
import { type Request, form, page, query, unreadable } from "../appos.ts";
import { fullPage, gone, isDraftId, quote } from "../flow_view.ts";
import { addOns, parseChoice, shownTemplate } from "./domain.ts";
import { saveUc } from "./save_usecase.ts";
import { showUc } from "./show_usecase.ts";
import * as view from "./view.ts";

const TITLE = "Your workspace";

export function show(req: Request): void {
  const id = query(req, "draft");
  if (!isDraftId(id)) return gone();
  showPage(req, id, "");
}

export function save(req: Request): void {
  const id = form(req, "draft");
  if (!isDraftId(id)) return gone();
  const choice = parseChoice(req.form);
  const saved = saveUc(id, choice);
  if (saved.ok) {
    if (req.htmx) {
      // A switch or template changed: saved, and the price follows.
      page.setHtml("quote", quote(saved.draft).text);
      page.setHtml("configure_error", "");
    } else {
      page.redirect(flow.next(id, "configure"));
    }
    return;
  }
  switch (saved.error) {
    case "not-found":
      return gone();
    case "booked":
      return page.redirect(flow.link(id, "setup"));
    case "refused":
      page.setStatus(422);
      if (req.htmx) return page.setHtml("configure_error", saved.why);
      return showPage(req, id, saved.why);
  }
}

function showPage(req: Request, id: string, error: string): void {
  const shown = showUc(id);
  if (shown.ok) {
    const t: view.Page = {
      base: req.base,
      draft: shown.draft,
      addOns: addOns(shown.catalog, shown.draft),
      catalog: shown.catalog,
      template: shownTemplate(shown.draft, shown.configured),
      error,
      back: flow.link(id, "choose"),
    };
    return fullPage(TITLE, view.page(t));
  }
  switch (shown.error) {
    case "not-found":
      return gone();
    case "booked":
      return page.redirect(flow.link(id, "setup"));
    case "no-package":
      return page.redirect(flow.link(id, "choose"));
    case "unreadable":
      return unreadable(shown);
  }
}

// The set-up module's handlers.

import * as flow from "../../bindings/flow.ts";
import { type Request, form, page, query, url } from "../appos.ts";
import { fullPage, gone, isDraftId } from "../flow_view.ts";
import { addUc } from "./add_usecase.ts";
import { showUc } from "./show_usecase.ts";
import * as view from "./view.ts";

const TITLE = "Your workspace is ready";

export function show(req: Request): void {
  const id = query(req, "draft");
  if (!isDraftId(id)) return gone();
  showPage(req, id, "", "", "");
}

export function add(req: Request): void {
  const id = form(req, "draft");
  if (!isDraftId(id)) return gone();
  const name = form(req, "name") ?? "";
  const email = form(req, "email") ?? "";
  const added = addUc(id, name, email);
  if (added.ok && !req.htmx) {
    // Without JavaScript: back to the page, which has the new link.
    return page.redirect(url(req, `/setup?draft=${id}`));
  }
  if (!added.ok) page.setStatus(422);
  const [typedName, typedEmail, error] = added.ok ? ["", "", ""] : [name.trim(), email.trim(), added.error];
  if (!req.htmx) return showPage(req, id, typedName, typedEmail, error);
  const shown = showUc(id);
  if (!shown.ok) return gone();
  page.setHtml("team", view.team({ base: req.base, booking: shown.booking, name: typedName, email: typedEmail, error }).text);
}

function showPage(req: Request, id: string, name: string, email: string, error: string): void {
  const shown = showUc(id);
  if (shown.ok) return fullPage(TITLE, view.page({ base: req.base, booking: shown.booking, name, email, error }));
  switch (shown.error) {
    case "not-found":
      return gone();
    case "not-booked":
      return page.redirect(flow.link(id, "review"));
  }
}

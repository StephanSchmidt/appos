// The hello module's handlers: parse the input, call usecases with the
// parsed values, and turn what they return, errors included, into the
// page. Errors are handled here and only here (see ERRORS in appos guide).

import { type Request, form, isId, notFound, page, param, style, title, unreadable, url } from "../appos.ts";
import { STYLE } from "../style.ts";
import { Name, type NameError } from "./domain.ts";
import { greetUc } from "./greet_usecase.ts";
import { listUc } from "./list_usecase.ts";
import { showUc } from "./show_usecase.ts";
import * as view from "./view.ts";

export function index(req: Request): void {
  fullPage(req, "", undefined);
}

export function greet(req: Request): void {
  const typed = form(req, "name") ?? "";
  const parsed = Name.parse(typed);
  if (!parsed.ok) return invalid(req, typed, parsed.error);
  greetUc(parsed.name, req.now);
  if (req.htmx) {
    // Only these parts change; the form keeps its text and focus.
    const t: view.Index = { base: req.base, history: listUc(), name: "", error: undefined };
    page.setHtml("greeting", view.greeting(t).text);
    page.setHtml("history", view.history(t).text);
    // Clears an error from an earlier try.
    page.setHtml("name_error", view.nameError(t).text);
  } else {
    // Without JavaScript: back to the page, which shows the new state.
    page.redirect(url(req, "/"));
  }
}

export function greeting(req: Request): void {
  // Text that isn't an id is as unknown as an id nobody has: 404.
  const id = param(req, "id");
  if (id === undefined || !isId(id)) return notFound();
  // Every case by name: a new kind of error doesn't type-check until this
  // handler decides what the visitor sees.
  const shown = showUc(id);
  if (shown.ok) {
    style(STYLE);
    title(`Hello, ${shown.greeting.name}`);
    page.setHtml("page", view.greetingPage({ base: req.base, greeting: shown.greeting }).text);
    return;
  }
  switch (shown.error) {
    case "not-found":
      return notFound();
    case "unreadable":
      return unreadable(shown);
  }
}

/** Input that doesn't parse: status 422 and the error next to the field.
 * With htmx only the error changes; without, the whole page comes back
 * with what was typed. */
function invalid(req: Request, typed: string, error: NameError): void {
  page.setStatus(422);
  if (req.htmx) {
    page.setHtml("name_error", view.nameError({ base: req.base, history: [], name: "", error }).text);
  } else {
    fullPage(req, typed, error);
  }
}

function fullPage(req: Request, name: string, error: NameError | undefined): void {
  style(STYLE);
  title("Hello");
  page.setHtml("page", view.index({ base: req.base, history: listUc(), name, error }).text);
}

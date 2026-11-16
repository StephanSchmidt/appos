// The public pages' handlers: the status page and one incident's page.
// Customers never see who posted what.

import { type Request, notFound, page, param, style, title, unreadable, url } from "../appos.ts";
import { type Utc, iso, parse, stamp } from "../time.ts";
import { STYLE } from "./assets.ts";
import type { Board } from "./board.ts";
import { boardUc } from "./board_usecase.ts";
import { overallSentence, overallStatus, parseId } from "./domain.ts";
import { showIncidentUc } from "./show_usecase.ts";
import * as templates from "./templates.ts";
import * as view from "./view.ts";

export function index(req: Request): void {
  const t = statusPage(req, boardUc(now(req)));
  head("Kitewell status");
  page.setHtml("page", templates.index(t).text);
}

/** The refresh every minute: only the live part changes. */
export function live(req: Request): void {
  if (!req.htmx) {
    page.redirect(url(req, "/"));
    return;
  }
  const t = statusPage(req, boardUc(now(req)));
  page.setHtml("live", templates.live(t).text);
}

export function incident(req: Request): void {
  const id = parseId(param(req, "id") ?? "");
  if (id === undefined) return notFound();
  const shown = showIncidentUc(id);
  if (shown.ok) {
    head(`${shown.incident.title} · Kitewell status`);
    page.setHtml("page", templates.incidentPage(req.base, view.incidentView(shown.incident, now(req))).text);
    return;
  }
  switch (shown.error) {
    case "not-found":
      return notFound();
    case "unreadable":
      return unreadable(shown);
  }
}

function statusPage(req: Request, board: Board): templates.Index {
  const overall = board.overall();
  const maintenance = [...board.maintenance("in-progress"), ...board.maintenance("upcoming")].map((m) => view.maintenanceView(m, board.now));
  return {
    base: req.base,
    overall: overallSentence(overall),
    overallKey: overallStatus(overall),
    checked: stamp(board.now),
    checkedIso: iso(board.now),
    open: board.openIncidents().map((i) => view.incidentView(i, board.now)),
    maintenance,
    groups: view.groups(board),
    months: view.pastMonths(board),
  };
}

/** The design and the title, for every full page of the app. */
export function head(text: string): void {
  style(STYLE);
  title(text);
}

/** The time of this request. `start` in lib.ts checked that it parses
 * before any handler runs. */
export function now(req: Request): Utc {
  const t = parse(req.now);
  if (t === undefined) throw new Error(`the request's time "${req.now}" is not RFC 3339, though start checked it`);
  return t;
}

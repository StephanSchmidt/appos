// Kitewell status, an AppOS app in TypeScript: a public status page that
// customer support keeps up to date itself, and the desk where support
// writes it.
//
// One module, status, plus time.ts, the UTC times every page shows. The
// module has the layers every module has:
//   index.ts            its routes and menu entry (MODULE)
//   web.ts, desk.ts     handlers: parse the input (422 and the message if
//                       it doesn't parse), call usecases with the parsed
//                       values, turn their results and errors (an unknown
//                       id: 404) into the page. Errors end here.
//   <name>_usecase.ts   one usecase per file, as a pair:
//                         openU     the logic; every IO it needs comes in
//                                   as a function parameter
//                         openUc    wires openU to the real IO in db.ts
//   db.ts               the IO: every kv call, with the keys and the JSON
//   domain.ts, board.ts the data and its rules, without IO
//   view.ts             what the pages show, worked out from the data
//   templates.ts        the pages, as html templates
// src/appos.ts is the platform's side: leave it to `appos sync`.

import type { Handler } from "../bindings/handler.ts";
import { type Module, type Request, dispatch, failed, page, register } from "./appos.ts";
import * as status from "./status/index.ts";
import { parse } from "./time.ts";

/** Every module of the app. */
const MODULES: Module[] = [status.MODULE];

export const handler: Handler = {
  init(): void {
    register(MODULES);
  },
  start(name: string, req: Request): void {
    // Handlers read the request's time (now in status/web.ts): checked
    // once here, so none of them has to fail on it.
    if (parse(req.now) === undefined) failed(`the request's time "${req.now}" is not RFC 3339`);
    else dispatch(MODULES, name, req);
    page.done();
  },
};

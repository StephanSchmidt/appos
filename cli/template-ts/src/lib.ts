// Hello world: an AppOS app in TypeScript. Change it into yours; `appos
// guide` explains the shell and its calls.
//
// An app is made of modules, one folder each under src/. This one has a
// single module, hello. A module owns its routes, its pages and its kv keys,
// and every module has the same layers:
//   index.ts            its routes and menu entries (MODULE)
//   web.ts              handlers: parse the input (422 and the message if
//                       it doesn't parse), call usecases with the parsed
//                       values, turn their results and errors (an unknown
//                       id: 404) into the page. Errors end here.
//   view.ts             the page and its parts, as html templates
//   <name>_usecase.ts   one usecase per file, as a pair:
//                         greetU    the logic; every IO it needs comes in
//                                   as a function parameter (addGreeting,
//                                   log), so tests pass fakes
//                         greetUc   wires greetU to the real IO in db.ts
//                       A usecase that can fail says how in its result
//                       (show_usecase.ts: { ok: false, error: "not-found" }).
//   db.ts               the IO: every kv call, with the keys
//   domain.ts           the data and its rules, without IO; also the checks
//                       of what visitors type (Name.parse)
// Tests sit next to the code (domain.test.ts); `npm test` runs them under
// Node, without the shell, after `appos build` (make check does both).
// src/appos.ts is the platform's side: leave it to `appos sync`.

import type { Handler } from "../bindings/handler.ts";
import { type Module, type Request, dispatch, page, register } from "./appos.ts";
import * as hello from "./hello/index.ts";

/** Every module of the app: a new module adds its MODULE here. */
const MODULES: Module[] = [hello.MODULE];

export const handler: Handler = {
  init(): void {
    register(MODULES);
  },
  start(name: string, req: Request): void {
    dispatch(MODULES, name, req);
    page.done();
  },
};

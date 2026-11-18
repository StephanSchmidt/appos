// Kitewell's workspace app, owned by product: steps 2 and 5 of the booking
// flow (request.md). The booking functions (kitewell:booking) keep the
// prices and the bookings; this app decides what the customer sees and
// what is preselected.
//
// Two modules, one folder each, with the layers of every AppOS app in
// TypeScript (`appos guide`):
//   configure/   step 2: add-ons and the first project's template
//   setup/       step 5: the workspace, and the team with invite links
// flow_view.ts holds what both pages share (the steps, the quote), style.ts
// Kitewell's look. src/appos.ts is the platform's side: leave it to
// `appos sync`.

import type { Handler } from "../bindings/handler.ts";
import { type Module, type Request, dispatch, page, register } from "./appos.ts";
import * as configure from "./configure/index.ts";
import * as setup from "./setup/index.ts";

const MODULES: Module[] = [configure.MODULE, setup.MODULE];

export const handler: Handler = {
  init(): void {
    register(MODULES);
  },
  start(name: string, req: Request): void {
    dispatch(MODULES, name, req);
    page.done();
  },
};

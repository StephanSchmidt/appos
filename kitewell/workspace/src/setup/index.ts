// The set-up module: step 5, right after the booking. The workspace is
// there, and the customer adds their team. The platform links here with
// ?draft=<id>.

import { type Module, get, post } from "../appos.ts";
import * as web from "./web.ts";

export const MODULE: Module = {
  routes: [get("/setup", web.show), post("/setup", web.add)],
  menu: [],
};

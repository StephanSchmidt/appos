// The configure module: step 2 of the booking flow, the workspace the
// customer starts with. The platform links here with ?draft=<id> while
// this app serves; otherwise its own plain page takes over.

import { type Module, get, post } from "../appos.ts";
import * as web from "./web.ts";

export const MODULE: Module = {
  routes: [get("/configure", web.show), post("/configure", web.save)],
  menu: [],
};

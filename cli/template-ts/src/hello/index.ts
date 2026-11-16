// The hello module: say hello, and see who said hello before. Other modules
// reach it only through this file; its data (db.ts) stays private to it.

import { type Module, get, post } from "../appos.ts";
import * as web from "./web.ts";

/** The module's routes and menu entries. Paths are below the app's base;
 * `:name` segments arrive as `param(req, "name")`. */
export const MODULE: Module = {
  routes: [get("/", web.index), post("/greet", web.greet), get("/greetings/:id", web.greeting)],
  menu: [["Hello", "/"]],
};

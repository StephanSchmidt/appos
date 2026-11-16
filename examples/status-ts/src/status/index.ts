// The status module: the public status page customers read, and the desk
// where support writes it.
//
//   web.ts        the public pages: the status page and one incident
//   desk.ts       the desk: open incidents, post updates, fix typos, set
//                 parts, schedule maintenance
//   view.ts       the words, times and colours the pages show
//   templates.ts  the pages and their parts, as html templates
//   board.ts      each part's status now, the 90 days and the uptime
//   domain.ts     the data and its rules, and the checks of what the desk
//                 types
//   seed.ts       the starting history
//   db.ts         every kv call

import { type Module, get, post } from "../appos.ts";
import * as desk from "./desk.ts";
import * as web from "./web.ts";

/** The module's routes and menu entry. The desk has no menu entry:
 * customers don't need it, support bookmarks it. */
export const MODULE: Module = {
  routes: [
    get("/", web.index),
    get("/live", web.live),
    get("/incidents/:id", web.incident),
    get("/desk", desk.index),
    get("/desk/compose", desk.compose),
    post("/desk/incidents", desk.open),
    get("/desk/incidents/:id", desk.incident),
    post("/desk/incidents/:id/updates", desk.postUpdateHandler),
    post("/desk/incidents/:id/updates/:n", desk.editUpdate),
    post("/desk/parts", desk.setParts),
    post("/desk/maintenance", desk.schedule),
    get("/desk/maintenance/:id", desk.maintenance),
    post("/desk/maintenance/:id/updates", desk.postMaintenanceUpdate),
  ],
  menu: [["Status", "/"]],
};

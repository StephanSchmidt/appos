// The starting history, so the page doesn't look brand new: one resolved
// email incident on Nov 6 and the read-only maintenance on Nov 22. Stored
// once, on the first request (see board_usecase.ts). Plain TypeScript, no
// IO.

import { type Utc, at } from "../time.ts";
import type { Change, Incident, Maintenance, Stage, Status, Update } from "./domain.ts";
import { newId } from "../appos.ts";

export interface Seed {
  incident: Incident;
  changes: Change[];
  maintenance: Maintenance;
}

export function seed(now: Utc): Seed {
  const id = newId();
  const update = (stage: Stage, hour: number, minute: number, message: string, nextUpdate: number | null): Update => ({
    stage,
    message,
    at: at(2026, 11, 6, hour, minute),
    by: "amara",
    next_update: nextUpdate,
    edits: [],
  });
  const incident: Incident = {
    id,
    title: "Delayed email notifications",
    opened_at: at(2026, 11, 6, 9, 12),
    resolved_at: at(2026, 11, 6, 10, 47),
    impact: [{ part: "email", now: "operational", worst: "partial" }],
    updates: [
      update(
        "investigating",
        9,
        12,
        "We're looking into reports that email notifications are arriving late or not at all. We'll post an update within 30 minutes.",
        30,
      ),
      update(
        "identified",
        9,
        40,
        "We've found the cause of the delayed emails and are working on a fix. No emails are lost: delayed notifications will be sent once the fix is in place.",
        30,
      ),
      update(
        "monitoring",
        10,
        15,
        "A fix is in place and we're watching the results. Delayed emails are going out now, and new notifications arrive on time for most customers.",
        30,
      ),
      update(
        "resolved",
        10,
        47,
        "This is resolved: email notifications arrive on time again, and all delayed emails have been delivered. We're sorry for the trouble.",
        null,
      ),
    ],
  };
  const change = (status: Status, hour: number, minute: number): Change => ({
    part: "email",
    status,
    at: at(2026, 11, 6, hour, minute),
    by: "amara",
    incident: id,
  });
  const maintenance: Maintenance = {
    id: newId(),
    title: "Kitewell will be read-only for up to an hour",
    parts: ["web-app", "ios", "android", "api"],
    starts: at(2026, 11, 22, 2, 0),
    ends: at(2026, 11, 22, 3, 0),
    notice:
      "During the window you can open and view everything in Kitewell, on the web, in the iOS and Android apps and through the API, but you can't make changes: saving, creating and deleting won't work. Please plan changes for before 02:00 or after 03:00 UTC.",
    by: "amara",
    announced_at: now,
    updates: [],
  };
  return { incident, changes: [change("partial", 9, 12), change("operational", 10, 47)], maintenance };
}

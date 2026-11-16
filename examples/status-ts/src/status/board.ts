// Everything on the page at one moment, and the rules that read it: each
// part's status now (planned maintenance included), the worst state for the
// sentence at the top, the last 90 days and the uptime. Plain TypeScript,
// no IO.

import { DAY, type Utc, dayStart } from "../time.ts";
import {
  type Change,
  type Incident,
  type Maintenance,
  type Overall,
  type PartId,
  type Phase,
  type Status,
  PARTS,
  STATUSES,
  affects,
  ended,
  isDown,
  isOpen,
  maintenanceAffects,
  overallOf,
  phase,
  severity,
  worse,
  worst,
} from "./domain.ts";

/** The days the page shows per part, today included. */
export const DAYS = 90;

/** One day of one part. */
export interface Day {
  /** Midnight UTC. */
  start: Utc;
  /** The worst it was that day. */
  worst: Status;
  /** Seconds spent in each status, by severity. */
  seconds: number[];
  /** What happened: the incidents and maintenance that touched the part. */
  events: string[];
}

/** A stretch of time with one status. */
interface Span {
  from: Utc;
  to: Utc;
  status: Status;
}

/** A moment where the stored status changes, the number of maintenance
 * windows going on changes, or both. */
interface Boundary {
  at: Utc;
  changed: Status | undefined;
  delta: number;
}

export class Board {
  readonly now: Utc;
  /** Newest first. */
  private readonly incidents: Incident[];
  /** Oldest first, none after now. */
  private readonly changes: Change[];
  /** By start. */
  private readonly maintenances: Maintenance[];

  constructor(now: Utc, incidents: Incident[], changes: Change[], maintenance: Maintenance[]) {
    this.now = now;
    // Array.sort is stable, like Rust's sort_by_key.
    this.incidents = incidents.slice().sort((a, b) => b.opened_at - a.opened_at);
    this.changes = changes.filter((c) => c.at <= now).sort((a, b) => a.at - b.at);
    this.maintenances = maintenance.slice().sort((a, b) => a.starts - b.starts);
  }

  /** The status the desk set last, without maintenance. */
  stored(part: PartId): Status {
    return storedStatus(this.changes, part);
  }

  /** Maintenance going on now that affects the part. */
  maintenanceNow(part: PartId): Maintenance | undefined {
    return this.maintenances.find((m) => maintenanceAffects(m, part) && phase(m, this.now) === "in-progress");
  }

  /** What customers see: the desk's status, and Under maintenance during a
   * window unless something worse is going on. */
  status(part: PartId): Status {
    const stored = this.stored(part);
    return this.maintenanceNow(part) === undefined ? stored : worse(stored, "maintenance");
  }

  overall(): Overall {
    return overallOf(worst(PARTS.map((p) => this.status(p))));
  }

  openIncidents(): Incident[] {
    return this.incidents.filter((i) => isOpen(i));
  }

  /** Newest first. */
  resolvedIncidents(): Incident[] {
    return this.incidents.filter((i) => !isOpen(i));
  }

  maintenance(p: Phase): Maintenance[] {
    return this.maintenances.filter((m) => phase(m, this.now) === p);
  }

  /** The first day of the 90. */
  windowStart(): Utc {
    return dayStart(this.now) - (DAYS - 1) * DAY;
  }

  /** The last 90 days of a part, oldest first. */
  days(part: PartId): Day[] {
    const spans = this.spans(part);
    const days: Day[] = [];
    for (let i = 0; i < DAYS; i++) {
      const start = this.windowStart() + i * DAY;
      const end = Math.min(start + DAY, this.now);
      const seconds = [0, 0, 0, 0, 0];
      for (const span of spans) seconds[severity(span.status)] += overlap(span.from, span.to, start, end);
      const w = worst(STATUSES.filter((s) => seconds[severity(s)] > 0));
      days.push({ start, worst: w, seconds, events: this.events(part, start, end) });
    }
    return days;
  }

  /** The share of the 90 days without a partial or major outage, in
   * percent. Today counts up to now. */
  uptime(part: PartId): number {
    const start = this.windowStart();
    const end = this.now;
    const total = end - start;
    if (total <= 0) return 100;
    let down = 0;
    for (const s of this.spans(part)) if (isDown(s.status)) down += overlap(s.from, s.to, start, end);
    return (100 * (total - down)) / total;
  }

  /** The part's statuses over time: what the desk set, and maintenance
   * windows that have begun. */
  private spans(part: PartId): Span[] {
    const start = this.windowStart();
    // Each boundary changes the stored status, the number of active
    // maintenance windows, or both. One second belongs to one status.
    const boundaries = new Map<Utc, Boundary>();
    const boundary = (at: Utc): Boundary => {
      let b = boundaries.get(at);
      if (b === undefined) {
        b = { at, changed: undefined, delta: 0 };
        boundaries.set(at, b);
      }
      return b;
    };
    let status: Status = "operational";
    for (const change of this.changes) {
      if (change.part !== part) continue;
      if (change.at <= start) status = change.status;
      else if (change.at < this.now) boundary(change.at).changed = change.status;
    }
    let active = 0;
    for (const m of this.maintenances) {
      if (!(maintenanceAffects(m, part) && m.starts < this.now && m.ends > start && m.ends > m.starts)) continue;
      if (m.starts <= start) active += 1;
      else boundary(m.starts).delta += 1;
      if (m.ends < this.now) boundary(m.ends).delta -= 1;
    }
    const visible = (s: Status, n: number): Status => (n > 0 ? worse(s, "maintenance") : s);
    const spans: Span[] = [];
    let from = start;
    const ordered = [...boundaries.values()].sort((a, b) => a.at - b.at);
    for (const b of ordered) {
      spans.push({ from, to: b.at, status: visible(status, active) });
      from = b.at;
      if (b.changed !== undefined) status = b.changed;
      active += b.delta;
    }
    spans.push({ from, to: this.now, status: visible(status, active) });
    return spans;
  }

  /** The titles of incidents and maintenance that touched the part between
   * `start` and `end`. */
  private events(part: PartId, start: Utc, end: Utc): string[] {
    const events: string[] = [];
    for (let i = this.incidents.length - 1; i >= 0; i--) {
      const incident = this.incidents[i];
      if (affects(incident, part) && incident.opened_at < end && ended(incident, this.now) > start) events.push(incident.title);
    }
    for (const m of this.maintenances) {
      if (maintenanceAffects(m, part) && m.starts < end && m.starts < this.now && m.ends > start) events.push(`Maintenance: ${m.title}`);
    }
    return events;
  }
}

/** A part's status as the desk set it last. `changes` are oldest first: of
 * two in the same second, the later one in the list wins. */
export function storedStatus(changes: Change[], part: PartId): Status {
  let last: Change | undefined = undefined;
  for (const c of changes) if (c.part === part && (last === undefined || c.at >= last.at)) last = c;
  return last === undefined ? "operational" : last.status;
}

/** Seconds that [a0, a1) and [b0, b1) share. */
function overlap(a0: Utc, a1: Utc, b0: Utc, b1: Utc): number {
  return Math.max(Math.min(a1, b1) - Math.max(a0, b0), 0);
}

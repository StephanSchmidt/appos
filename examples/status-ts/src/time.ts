// Time in UTC. Customers are all over the world, so everything the app
// stores and shows is UTC: seconds since 1970 inside, "Nov 13, 14:05 UTC"
// for people. Plain TypeScript, no IO.

/** A moment, as seconds since 1970-01-01 00:00 UTC. */
export type Utc = number;

export const MINUTE = 60;
export const HOUR = 60 * MINUTE;
export const DAY = 24 * HOUR;

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTH_NAMES = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/** A month: year and month 1-12. */
export interface Month {
  year: number;
  month: number;
}

/** A moment split into calendar fields, in UTC. */
interface Civil {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
  weekday: number;
}

/** Floor division, like Rust's div_euclid for a positive divisor. */
function div(a: number, b: number): number {
  return Math.floor(a / b);
}

/** The remainder of floor division, never negative for a positive divisor. */
function mod(a: number, b: number): number {
  return a - Math.floor(a / b) * b;
}

/** A moment from calendar fields, e.g. `at(2026, 11, 6, 9, 12)`. */
export function at(year: number, month: number, day: number, hour: number, minute: number): Utc {
  return daysFromCivil(year, month, day) * DAY + hour * HOUR + minute * MINUTE;
}

/** Parses RFC 3339 as the shell sends it: "2026-11-11T14:05:09+01:00",
 * "2026-11-11T13:05:09.25Z". */
export function parse(text: string): Utc | undefined {
  const parts = splitDateTime(text.trim());
  if (parts === undefined) return undefined;
  const [date, time] = parts;
  const days = parseDate(date);
  const hour = number(time.slice(0, 2), 0, 23);
  const minute = number(time.slice(3, 5), 0, 59);
  const second = number(time.slice(6, 8), 0, 60);
  if (days === undefined || hour === undefined || minute === undefined || second === undefined) return undefined;
  if (time.slice(2, 3) !== ":" || time.slice(5, 6) !== ":") return undefined;
  let zone = time.slice(8);
  if (zone.startsWith(".")) {
    const fraction = zone.slice(1);
    if (fraction === "" || !isDigit(fraction.charCodeAt(0))) return undefined;
    let i = 0;
    while (i < fraction.length && isDigit(fraction.charCodeAt(i))) i++;
    zone = fraction.slice(i);
  }
  let offset = 0;
  if (zone !== "Z" && zone !== "z") {
    let sign: number;
    if (zone.startsWith("+")) sign = 1;
    else if (zone.startsWith("-")) sign = -1;
    else return undefined;
    if (zone.length !== 6 || zone.slice(3, 4) !== ":") return undefined;
    const hours = number(zone.slice(1, 3), 0, 23);
    const minutes = number(zone.slice(4, 6), 0, 59);
    if (hours === undefined || minutes === undefined) return undefined;
    offset = sign * (hours * HOUR + minutes * MINUTE);
  }
  return days * DAY + hour * HOUR + minute * MINUTE + second - offset;
}

/** Parses what a date-time field sends, read as UTC: "2026-11-22T02:00",
 * also with seconds or a space instead of the T. */
export function parseInput(text: string): Utc | undefined {
  const parts = splitDateTime(text.trim());
  if (parts === undefined) return undefined;
  const days = parseDate(parts[0]);
  const time = parts[1].trim();
  const hour = number(time.slice(0, 2), 0, 23);
  if (days === undefined || hour === undefined || time.length < 3 || time.slice(2, 3) !== ":") return undefined;
  const minute = number(time.slice(3, 5), 0, 59);
  if (minute === undefined || time.length < 5) return undefined;
  const rest = time.slice(5);
  let second = 0;
  if (rest !== "") {
    if (!rest.startsWith(":")) return undefined;
    const s = number(rest.slice(1), 0, 59);
    if (s === undefined) return undefined;
    second = s;
  }
  return days * DAY + hour * HOUR + minute * MINUTE + second;
}

/** Splits at the first 'T', 't' or space. */
function splitDateTime(text: string): [string, string] | undefined {
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "T" || c === "t" || c === " ") return [text.slice(0, i), text.slice(i + 1)];
  }
  return undefined;
}

/** Midnight UTC of this moment's day. */
export function dayStart(t: Utc): Utc {
  return div(t, DAY) * DAY;
}

function civil(t: Utc): Civil {
  const days = div(t, DAY);
  const secs = mod(t, DAY);
  const [year, month, day] = civilFromDays(days);
  return { year, month, day, hour: Math.trunc(secs / HOUR), minute: Math.trunc(mod(secs, HOUR) / MINUTE), weekday: mod(days + 4, 7) };
}

function two(n: number): string {
  return String(n).padStart(2, "0");
}

/** "Nov 13, 14:05 UTC" */
export function stamp(t: Utc): string {
  return `${shortDate(t)}, ${clock(t)} UTC`;
}

/** "14:05" */
export function clock(t: Utc): string {
  const c = civil(t);
  return `${two(c.hour)}:${two(c.minute)}`;
}

/** "Nov 13" */
export function shortDate(t: Utc): string {
  const c = civil(t);
  return `${MONTHS[c.month - 1]} ${c.day}`;
}

/** "Nov 13, 2026" */
export function date(t: Utc): string {
  const c = civil(t);
  return `${MONTHS[c.month - 1]} ${c.day}, ${c.year}`;
}

/** "Sun, Nov 22" */
export function weekdayDate(t: Utc): string {
  return `${WEEKDAYS[civil(t).weekday]}, ${shortDate(t)}`;
}

/** The month of a moment. */
export function month(t: Utc): Month {
  const c = civil(t);
  return { year: c.year, month: c.month };
}

/** The value of a date-time field: "2026-11-22T02:00". */
export function input(t: Utc): string {
  const c = civil(t);
  return `${String(c.year).padStart(4, "0")}-${two(c.month)}-${two(c.day)}T${two(c.hour)}:${two(c.minute)}`;
}

/** For <time datetime>: "2026-11-22T02:00:00Z". */
export function iso(t: Utc): string {
  return `${input(t)}:${two(mod(t, MINUTE))}Z`;
}

/** "November 2026" */
export function monthName(m: Month): string {
  return `${MONTH_NAMES[m.month - 1]} ${m.year}`;
}

/** The month before: 2026-01 -> 2025-12. */
export function previousMonth(m: Month): Month {
  return m.month === 1 ? { year: m.year - 1, month: 12 } : { year: m.year, month: m.month - 1 };
}

/** "25 minutes ago", "in 3 days", "just now". */
export function ago(then: Utc, now: Utc): string {
  const diff = now - then;
  const n = Math.abs(diff);
  let amount: string;
  if (n < MINUTE) return "just now";
  else if (n < HOUR) amount = plural(Math.trunc(n / MINUTE), "minute");
  else if (n < DAY) amount = plural(Math.trunc(n / HOUR), "hour");
  else amount = plural(Math.trunc(n / DAY), "day");
  return diff > 0 ? `${amount} ago` : `in ${amount}`;
}

/** How long something lasted: "1 h 35 min", "25 min", "2 days 3 h". */
export function duration(secs: number): string {
  if (secs < MINUTE) return "under a minute";
  const total = Math.trunc((secs + 30) / MINUTE);
  const days = Math.trunc(total / 1440);
  const hours = Math.trunc((total % 1440) / 60);
  const minutes = total % 60;
  if (days === 0 && hours === 0) return `${minutes} min`;
  if (days === 0 && minutes === 0) return `${hours} h`;
  if (days === 0) return `${hours} h ${minutes} min`;
  if (hours === 0) return plural(days, "day");
  return `${plural(days, "day")} ${hours} h`;
}

function plural(n: number, unit: string): string {
  return n === 1 ? `1 ${unit}` : `${n} ${unit}s`;
}

/** "2026-11-22" as days since 1970. */
function parseDate(text: string): number | undefined {
  if (text.length !== 10 || text.slice(4, 5) !== "-" || text.slice(7, 8) !== "-") return undefined;
  const year = number(text.slice(0, 4), 1970, 9999);
  const month = number(text.slice(5, 7), 1, 12);
  const day = number(text.slice(8, 10), 1, 31);
  if (year === undefined || month === undefined || day === undefined) return undefined;
  if (day > daysInMonth(year, month)) return undefined;
  return daysFromCivil(year, month, day);
}

function isDigit(code: number): boolean {
  return code >= 48 && code <= 57;
}

/** ASCII digits only, between min and max. */
function number(text: string, min: number, max: number): number | undefined {
  if (text === "") return undefined;
  for (let i = 0; i < text.length; i++) if (!isDigit(text.charCodeAt(i))) return undefined;
  const n = Number.parseInt(text, 10);
  return n >= min && n <= max ? n : undefined;
}

function daysInMonth(year: number, month: number): number {
  if (month === 2) return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28;
  if (month === 4 || month === 6 || month === 9 || month === 11) return 30;
  return 31;
}

/** Days since 1970 of a calendar date (Howard Hinnant's algorithm). */
function daysFromCivil(year: number, month: number, day: number): number {
  const y = month <= 2 ? year - 1 : year;
  const era = div(y, 400);
  const yoe = y - era * 400;
  const mp = (month + 9) % 12;
  const doy = Math.trunc((153 * mp + 2) / 5) + day - 1;
  const doe = yoe * 365 + Math.trunc(yoe / 4) - Math.trunc(yoe / 100) + doy;
  return era * 146097 + doe - 719468;
}

/** The calendar date of days since 1970: [year, month, day]. */
function civilFromDays(days: number): [number, number, number] {
  const z = days + 719468;
  const era = div(z, 146097);
  const doe = z - era * 146097;
  const yoe = Math.trunc((doe - Math.trunc(doe / 1460) + Math.trunc(doe / 36524) - Math.trunc(doe / 146096)) / 365);
  const doy = doe - (365 * yoe + Math.trunc(yoe / 4) - Math.trunc(yoe / 100));
  const mp = Math.trunc((5 * doy + 2) / 153);
  const day = doy - Math.trunc((153 * mp + 2) / 5) + 1;
  const month = mp < 10 ? mp + 3 : mp - 9;
  const year = yoe + era * 400 + (month <= 2 ? 1 : 0);
  return [year, month, day];
}

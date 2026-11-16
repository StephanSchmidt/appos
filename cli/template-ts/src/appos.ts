// The app's side of AppOS: the bindings `appos build` generates from wit/
// (the shell's contract, in bindings/) plus helpers: routes and modules,
// links, HTML templates, kv with JSON, ids. `appos new` writes this file and
// `appos sync` brings it up to date with the `appos` you have, so keep your
// own code elsewhere.
//
// An app has two functions. `init` runs once and registers its modules'
// routes and menu entries (`register(MODULES)`). `start` runs per request
// (`dispatch(MODULES, ...)`): it sets named HTML with `page.setHtml`
// ("page" for the whole page, any other name for an htmx update of the
// element with that id) and calls `page.done()`.
//
// HTML comes from `html` templates: html`<p>${text}</p>` escapes every
// value, unless it is already Html (another template, a list of them, or
// `raw`).

import * as kvCalls from "../bindings/kv.ts";
import * as log from "../bindings/log.ts";
import * as page from "../bindings/page.ts";
import * as registry from "../bindings/registry.ts";
import type { Request } from "../bindings/types.ts";

export type { Request };
export * as kv from "../bindings/kv.ts";
export * as log from "../bindings/log.ts";
export * as page from "../bindings/page.ts";

// Routes and modules

/** A route: method, path and the function that handles it. `path` is below
 * the app's base; `:name` segments arrive as `param(req, "name")`. */
export interface Route {
  method: string;
  path: string;
  handler: (req: Request) => void;
}

export function get(path: string, handler: (req: Request) => void): Route {
  return { method: "GET", path, handler };
}

export function post(path: string, handler: (req: Request) => void): Route {
  return { method: "POST", path, handler };
}

/** A part of the app: its routes and its menu entries [label, path]. The
 * app lists its modules once, `const MODULES = [hello.MODULE]`, for both
 * `register` and `dispatch`. */
export interface Module {
  routes: Route[];
  menu: [string, string][];
}

/** The name the shell hands to `start`, e.g. "POST /greet": unique as long
 * as method and path are. */
function routeName(route: Route): string {
  return `${route.method} ${route.path}`;
}

/** Registers every module's routes and menu entries, during `init`. */
export function register(modules: Module[]): void {
  for (const module of modules) {
    for (const route of module.routes) registry.addRoute(route.method, route.path, routeName(route));
    for (const [label, path] of module.menu) registry.addMenu(label, path);
  }
}

/** Runs the route the shell picked for this request, from all modules'
 * routes; a 404 page if there is none. */
export function dispatch(modules: Module[], handler: string, req: Request): void {
  for (const module of modules) {
    for (const route of module.routes) {
      if (routeName(route) === handler) {
        route.handler(req);
        return;
      }
    }
  }
  notFound();
}

// The request

function find(pairs: [string, string][], key: string): string | undefined {
  for (const [k, v] of pairs) if (k === key) return v;
  return undefined;
}

/** A route parameter, e.g. `:id`. */
export function param(req: Request, name: string): string | undefined {
  return find(req.params, name);
}

/** A query parameter. */
export function query(req: Request, name: string): string | undefined {
  return find(req.query, name);
}

/** A form field of a POST. */
export function form(req: Request, name: string): string | undefined {
  return find(req.form, name);
}

/** A link to a path inside this app: url(req, "/desk") -> "/events/desk",
 * url(req, "/") -> "/events". */
export function url(req: Request, path: string): string {
  const rest = path.startsWith("/") ? path.slice(1) : path;
  const separator = rest === "" || rest.startsWith("?") ? "" : "/";
  return req.base + separator + rest;
}

/** A link with a query string, values encoded:
 * urlWith(req, "/", [["q", "a b"]]) -> "/events?q=a%20b". */
export function urlWith(req: Request, path: string, query: [string, string][]): string {
  return url(req, path) + "?" + query.map(([k, v]) => encode(k) + "=" + encode(v)).join("&");
}

/** The same link with the site's origin, e.g. for QR codes and emails. */
export function absoluteUrl(req: Request, path: string): string {
  return req.origin + url(req, path);
}

const HEX = "0123456789ABCDEF";
const encoder = new TextEncoder();

/** Percent-encodes a query value: everything but A-Z a-z 0-9 - _ . ~ */
export function encode(value: string): string {
  let out = "";
  for (const b of encoder.encode(value)) {
    const unreserved = (b >= 48 && b <= 57) || (b >= 65 && b <= 90) || (b >= 97 && b <= 122) || b === 45 || b === 95 || b === 46 || b === 126;
    out += unreserved ? String.fromCharCode(b) : "%" + HEX[b >> 4] + HEX[b & 15];
  }
  return out;
}

// HTML

/** HTML-escapes text. */
export function escape(s: string): string {
  let out = "";
  for (const c of s) {
    if (c === "&") out += "&#38;";
    else if (c === "<") out += "&#60;";
    else if (c === ">") out += "&#62;";
    else if (c === '"') out += "&#34;";
    else if (c === "'") out += "&#39;";
    else out += c;
  }
  return out;
}

/** HTML that is safe to send as is. */
export class Html {
  readonly text: string;
  constructor(text: string) {
    this.text = text;
  }
}

export type Value = string | number | Html | Html[];

/** A template: html`<p>${text}</p>`. Text and numbers are escaped; Html
 * (another template, a list of them, `raw`) is inserted as is. */
export function html(strings: TemplateStringsArray, ...values: Value[]): Html {
  let out = strings[0];
  for (let i = 0; i < values.length; i++) {
    const v = values[i];
    if (v instanceof Html) out += v.text;
    else if (Array.isArray(v)) for (const h of v) out += h.text;
    else out += escape(String(v));
    out += strings[i + 1];
  }
  return new Html(out);
}

/** Markup that is already HTML, inserted unescaped. */
export function raw(text: string): Html {
  return new Html(text);
}

/** Nothing: for the empty branch of `cond ? html`...` : empty`. */
export const empty = new Html("");

/** Adds CSS to the page's <head>, e.g. the app's design. */
export function style(css: string): void {
  page.addHeader(`<style>${css}</style>`);
}

/** Sets the page's <title>, escaped like the Rust SDK's (&amp;). */
export function title(text: string): void {
  let out = "";
  for (const c of text) {
    if (c === "&") out += "&amp;";
    else if (c === "<") out += "&lt;";
    else if (c === ">") out += "&gt;";
    else if (c === '"') out += "&quot;";
    else if (c === "'") out += "&#39;";
    else out += c;
  }
  page.addHeader(`<title>${out}</title>`);
}

/** Status 500 and a page saying the page can't be shown; `why` goes to the
 * log, not the page. For what the app can't work with, like stored data
 * that isn't JSON (`unreadable`). */
export function failed(why: string): void {
  log.warn(why);
  page.setStatus(500);
  title("Can't be shown");
  page.setHtml(
    "page",
    '<h1 style="font-family: system-ui; text-align: center; margin: 4rem 1rem">This page can\'t be shown right now</h1>',
  );
}

/** Answers stored data the app can't read: `failed` with the key and why. */
export function unreadable(u: Unreadable): void {
  failed(u.why);
}

/** A 404 page. */
export function notFound(): void {
  page.setStatus(404);
  title("Not found");
  page.setHtml("page", '<h1 style="font-family: system-ui; text-align: center; margin: 4rem 1rem">Not found</h1>');
}

// kv with JSON

/** The most `scanJson` returns in one call. */
export const PAGE = 100;

/** Stored text that isn't JSON. Stored data can be anything: another
 * version of the app, a person or a bug may have written it. A usecase
 * returns it as it is, and the handler answers with `unreadable`. */
export type Unreadable = { ok: false; error: "unreadable"; why: string };

/** What `getJson` read: the value, undefined if there is none. */
export type Read<T> = { ok: true; value: T | undefined } | Unreadable;

/** The value at `key`, parsed from JSON; Unreadable if it isn't JSON. */
export function getJson<T>(key: string): Read<T> {
  const text = kvCalls.get(key);
  if (text === undefined) return { ok: true, value: undefined };
  try {
    return { ok: true, value: JSON.parse(text) as T };
  } catch (e) {
    return { ok: false, error: "unreadable", why: `${key} is unreadable: ${e instanceof Error ? e.message : String(e)}` };
  }
}

/** Stores `value` at `key` as JSON. */
export function setJson<T>(key: string, value: T): void {
  kvCalls.set(key, JSON.stringify(value));
}

/** Keys starting with `prefix` and their values parsed from JSON, in key
 * order (newest first with `reverse` when keys are ids from `newId`), at
 * most `limit`; pass the last key as `after` for the next page. A value
 * that isn't JSON is logged and left out, so one bad entry doesn't take
 * the whole list down. */
export function scanJson<T>(prefix: string, after: string | undefined, limit: number, reverse: boolean): [string, T][] {
  const out: [string, T][] = [];
  for (const [key, text] of kvCalls.scan(prefix, after, limit, reverse)) {
    try {
      out.push([key, JSON.parse(text) as T]);
    } catch (e) {
      log.warn(`${key} is unreadable, left out: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  return out;
}

/** Every key starting with `prefix`, page by page. */
export function scanAllJson<T>(prefix: string, reverse: boolean): [string, T][] {
  const out: [string, T][] = [];
  let after: string | undefined = undefined;
  for (;;) {
    const page = kvCalls.scan(prefix, after, PAGE, reverse);
    for (const [key, text] of page) {
      try {
        out.push([key, JSON.parse(text) as T]);
      } catch (e) {
        log.warn(`${key} is unreadable, left out: ${e instanceof Error ? e.message : String(e)}`);
      }
    }
    if (page.length < PAGE) return out;
    after = page[page.length - 1][0];
  }
}

// Ids

const hex = "0123456789abcdef";
let lastId = new Uint8Array(16);
let lastMs = -1;

/** A new id: a UUID v7, "019a6f3c-...". It starts with the time, so ids
 * sort in the order they were made, also within one millisecond; as kv
 * keys, a scan lists them oldest first (newest first with `reverse`). */
export function newId(): string {
  const ms = Date.now();
  const b = new Uint8Array(16);
  if (ms <= lastMs) {
    // The same millisecond: the last id plus one.
    b.set(lastId);
    for (let i = 15; i >= 9; i--) {
      b[i] = (b[i] + 1) & 255;
      if (b[i] !== 0) break;
    }
  } else {
    crypto.getRandomValues(b);
    let t = ms;
    for (let i = 5; i >= 0; i--) {
      b[i] = t % 256;
      t = Math.floor(t / 256);
    }
    b[6] = (b[6] & 0x0f) | 0x70; // version 7
    b[8] = (b[8] & 0x3f) | 0x80; // variant
    b[9] = b[9] & 0x7f; // room to count up within the millisecond
    lastMs = ms;
  }
  lastId = b;
  let s = "";
  for (let i = 0; i < 16; i++) {
    if (i === 4 || i === 6 || i === 8 || i === 10) s += "-";
    s += hex[b[i] >> 4] + hex[b[i] & 15];
  }
  return s;
}

/** True if `text` is an id as `newId` makes them (any UUID, lowercase). */
export function isId(text: string): boolean {
  if (text.length !== 36) return false;
  for (let i = 0; i < 36; i++) {
    const c = text[i];
    const dash = i === 8 || i === 13 || i === 18 || i === 23;
    if (dash ? c !== "-" : !hex.includes(c)) return false;
  }
  return true;
}

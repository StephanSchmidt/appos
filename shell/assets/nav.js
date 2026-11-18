// The console's one navigation controller. Every page it swaps in, every
// change it sends and every live refresh goes through here, so the newest
// wins and nothing overtakes anything:
//
// - Each navigation (a link, a knob come to rest, a form, back) gets a
//   number; a page that arrives after a newer one started is dropped.
// - Changes (POSTs) go one after another; the same change again while it
//   is on its way is sent once. A page is asked for after the changes
//   before it landed, so it shows them.
// - A control waits on its own timer (a fader resting, a knob between
//   turns, the cursor before it selects). A change still waiting is sent
//   before any navigation; the cursor's select is dropped by one.
// - The live refresh waits while a control is held or waiting, someone
//   types, a page or change is on its way or the tab is hidden, then
//   catches up; a refresh of a page that has moved on is dropped. Focus
//   stays on the control it was on.
// - A change without an answer leaves the page as it was, takes back what
//   it showed in advance and says so in a toast.
import { con, $, focusKey, refocus, plain, typing } from "./dom.js";

let seq = 0; // navigations started; the newest is the one to show
let reading = null; // the page on its way, aborted by a newer navigation
let posts = Promise.resolve(); // the changes, one after another
const sending = new Set(); // changes queued or on their way
const timers = new Map(); // a control's name → its wait
let held = 0; // controls in the pointer's hand
let flying = 0; // navigations on their way
let quiet = []; // what waits for them to land
let behind = false; // a refresh waited
let refreshAt = -1; // the navigation a refresh started at
let refreshing = false; // a refresh is swapping in
let refreshFocus = null;
const pageHooks = [];
const refreshHooks = [];

const SWAPPED = 1;
const STALE = 2;
const NOT_A_PAGE = 3;

// ---- what other parts do when a page comes in ------------------------------

// `fn(main, how)` after each new #live; how.refreshed for the live
// refresh, how.keep for a list scroll to keep.
export function onPage(fn) {
  pageHooks.push(fn);
}
// `fn()` just before the live refresh swaps in.
export function onRefresh(fn) {
  refreshHooks.push(fn);
}
export function pageIn(main, how) {
  pageHooks.forEach((fn) => fn(main, how));
}

// ---- one timer per control ---------------------------------------------------

// Runs `fn` after `ms`, replacing what `name` waited for; a `change` is
// sent at once when a navigation comes first.
export function wait(name, ms, fn, change = false) {
  cancel(name, true);
  const t = setTimeout(() => {
    timers.delete(name);
    fn();
    catchUp();
  }, ms);
  timers.set(name, { t, fn, change });
}
export function cancel(name, quietly = false) {
  const w = timers.get(name);
  if (!w) return;
  clearTimeout(w.t);
  timers.delete(name);
  if (!quietly) catchUp();
}
function flush() {
  for (const [name, w] of timers) {
    if (!w.change) continue;
    clearTimeout(w.t);
    timers.delete(name);
    w.fn();
  }
}

// A control in the pointer's hand: no refresh redraws it meanwhile.
export function hold() {
  held++;
}
export function release() {
  held = Math.max(0, held - 1);
  catchUp();
}

// Resolves once no navigation is on its way.
export function settled() {
  return flying ? new Promise((r) => quiet.push(r)) : Promise.resolve();
}
function track(p) {
  flying++;
  p.finally(() => {
    if (--flying) return;
    const waiting = quiet;
    quiet = [];
    waiting.forEach((r) => r());
    // After what waited had its turn: it may navigate again.
    setTimeout(catchUp, 0);
  });
  return p;
}

function holding() {
  return held > 0 || timers.size > 0 || flying > 0 || document.hidden || typing();
}

// ---- navigations -------------------------------------------------------------

function begin() {
  cancel("select", true);
  flush();
  if (reading) reading.abort();
  reading = null;
  return ++seq;
}

// Shows the page at `url`: "push", "replace" or "none" (back, forward).
// Resolves to whether it swapped in. Without a console page there, a
// plain navigation.
export function go(url, how, opts = {}) {
  const mine = begin();
  const ctrl = new AbortController();
  reading = ctrl;
  const focus = focusKey(document.activeElement);
  return track(
    posts
      .then(() => fetch(url, { credentials: "same-origin", signal: ctrl.signal }))
      .then((r) => r.text().then((text) => swap(mine, r, text, how, focus, opts)))
      .then((out) => {
        if (out === NOT_A_PAGE && mine === seq) location.href = url;
        return out === SWAPPED;
      })
      .catch((err) => {
        if (err.name !== "AbortError" && mine === seq) failed();
        return false;
      })
      .finally(() => {
        if (reading === ctrl) reading = null;
      })
  );
}

// Sends a change; the page it comes back on swaps in. A slide in its
// form moves at once and back if the change got no answer.
export function send(action, body, form) {
  // Look only: the page made it inert.
  if (form && form.closest("[inert]")) return Promise.resolve(false);
  const key = action + "?" + body;
  if (sending.has(key)) return Promise.resolve(false);
  sending.add(key);
  const slide = form && form.querySelector(".c-slide:not(.dead)");
  flip(slide);
  const mine = begin();
  const focus = focusKey(document.activeElement);
  const back = () => {
    flip(slide);
    // Said on the page that shows once the navigations land.
    settled().then(failed);
    return false;
  };
  const done = posts.then(() =>
    fetch(action, { method: "POST", body, credentials: "same-origin" })
      .then((r) => r.text().then((text) => swap(mine, r, text, "push", focus, {})))
      .then((out) => (out === NOT_A_PAGE ? back() : out === SWAPPED))
      .catch(back)
      .finally(() => sending.delete(key))
  );
  posts = done;
  return track(done);
}

export function submit(form, submitter) {
  const body = new URLSearchParams(new FormData(form, submitter || undefined));
  const action = form.getAttribute("action");
  if ((form.getAttribute("method") || "get").toLowerCase() === "post") return send(action, body, form);
  return go(action + "?" + body, "push");
}

function swap(mine, res, text, how, focus, opts) {
  const doc = new DOMParser().parseFromString(text, "text/html");
  const next = doc.getElementById("live");
  const now = document.getElementById("live");
  if (!next || !now) return NOT_A_PAGE;
  if (mine !== seq) return STALE;
  const to = new URL(res.url, location.href);
  const target = to.pathname + to.search;
  if (how === "push" && target !== location.pathname + location.search) history.pushState(null, "", target);
  else if (how !== "none") history.replaceState(null, "", target);
  const updated = doc.getElementById("updated");
  const ours = document.getElementById("updated");
  if (updated && ours) ours.replaceWith(document.importNode(updated, true));
  if (doc.title) document.title = doc.title;
  const adopted = document.importNode(next, true);
  now.replaceWith(adopted);
  if (window.htmx) htmx.process(adopted);
  pageIn(adopted, { refreshed: false, keep: opts.keep ?? null });
  refocus(adopted, focus);
  return SWAPPED;
}

// A slide shows the other side, or back.
function flip(slide) {
  if (!slide) return;
  slide.classList.toggle("off");
  slide.setAttribute("aria-checked", String(slide.getAttribute("aria-checked") !== "true"));
}

// The toast for a change or page without an answer, in the shell's words.
function failed() {
  const c = con();
  const lcd = c && $(".c-lcd", c);
  if (!lcd) return;
  lcd.querySelectorAll(".c-toast").forEach((t) => t.remove());
  const toast = document.createElement("div");
  toast.className = "c-toast r";
  toast.setAttribute("role", "alert");
  const text = document.createElement("span");
  text.textContent = c.dataset.failed;
  toast.append(text);
  const head = $(".c-head", lcd);
  if (head) head.after(toast);
  else lcd.prepend(toast);
}

// ---- the live refresh (htmx, on the stream's "changed") ----------------------

function catchUp() {
  if (behind && !holding() && window.htmx) {
    behind = false;
    htmx.trigger("#refresh", "refresh");
  }
}

// It asks for the page the console is on now.
document.addEventListener("htmx:configRequest", (e) => {
  if (e.target.id === "refresh") e.detail.path = location.pathname + location.search;
});
document.addEventListener("htmx:beforeRequest", (e) => {
  if (e.target.id !== "refresh") return;
  if (holding()) {
    behind = true;
    e.preventDefault();
    return;
  }
  refreshAt = seq;
});
document.addEventListener("htmx:beforeSwap", (e) => {
  if (e.target.id !== "refresh") return;
  // The console moved on, or a control is in hand: this page would undo it.
  if (refreshAt !== seq || holding()) {
    behind = true;
    e.detail.shouldSwap = false;
    e.preventDefault();
    setTimeout(catchUp, 0);
    return;
  }
  refreshing = true;
  refreshFocus = focusKey(document.activeElement);
  refreshHooks.forEach((fn) => fn());
});
// Right as the new #live is in, before it is painted (htmx:load comes a
// settle later): the parts redo theirs and focus goes back. It fires once
// per swapped part, so only the first time.
document.addEventListener("htmx:oobAfterSwap", (e) => {
  if (e.target.id !== "live" || !refreshing) return;
  refreshing = false;
  pageIn(e.target, { refreshed: true, keep: null });
  refocus(e.target, refreshFocus);
  refreshFocus = null;
});
document.addEventListener("visibilitychange", catchUp);
document.addEventListener("focusout", () => setTimeout(catchUp, 0));

// ---- links and forms ---------------------------------------------------------

// A console link swaps its page in. On window, so the controls' own
// handlers on document (a row, a light) go first.
window.addEventListener("click", (e) => {
  if (!plain(e)) return;
  const a = e.target.closest && e.target.closest("#console a[href]");
  if (!a || a.target) return;
  const href = a.getAttribute("href");
  if (!href.includes("#console")) return;
  e.preventDefault();
  go(href.replace("#console", ""), "push");
});
document.addEventListener("submit", (e) => {
  const form = e.target;
  if (e.defaultPrevented || !form.closest || !form.closest("#console")) return;
  e.preventDefault();
  submit(form, e.submitter);
});
window.addEventListener("popstate", () => go(location.pathname + location.search, "none"));

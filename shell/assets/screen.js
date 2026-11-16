// The screen: the list's rows (the map's squares, the versions), the
// cursor on them and the App knob that follows it, Find, the hint line,
// the strip's window and the list's order held while the pointer is on it.
import { $, $$, con, list, clamp, fill, hint, plain } from "./dom.js";
import { knob, stops, show } from "./dial.js";
import { go, wait, cancel, onPage, onRefresh } from "./nav.js";

let pointing = false; // the pointer is on the list
let cursor = null; // the app (or row number) the cursor is on, through refreshes
let top = null; // the list's scroll before a refresh
let order = null; // its apps before a refresh, while pointed at

// What the cursor moves over, the hidden ones (Find) left out.
export function rows() {
  return $$("#console .c-lcd .c-list .c-row:not(.c-colhead), #console .c-lcd .c-sq").filter((r) => !r.hidden);
}

function current() {
  const all = rows();
  let i = all.findIndex((r) => r.classList.contains("cursor"));
  if (i < 0) i = all.findIndex((r) => r.classList.contains("sel") || r.classList.contains("cur"));
  return { all, i };
}

// The App knob shows the app with this id.
function follow(id) {
  const k = knob("app");
  const i = k ? stops(k).values.indexOf(id) : -1;
  if (i >= 0) show(k, i);
}

function reveal(row) {
  const l = list();
  if (!row || !l || !l.contains(row)) return;
  const at = row.offsetTop - l.offsetTop;
  if (at < l.scrollTop) l.scrollTop = at;
  else if (at + row.offsetHeight > l.scrollTop + l.clientHeight) l.scrollTop = at + row.offsetHeight - l.clientHeight;
}

function mark(row) {
  rows().forEach((r) => r.classList.remove("cursor"));
  row.classList.add("cursor");
  reveal(row);
  frame();
}

// The list row of an app, where a click selects it in place.
export function rowOf(id) {
  return rows().find((r) => r.dataset.app === id && r.dataset.href);
}

// Selects a row where it is: at once on screen, then its page, the list
// scrolled as it was.
export function choose(row) {
  $$("#console .c-list .c-row.sel").forEach((r) => {
    r.classList.remove("sel");
    const n = $(".c-name", r);
    if (n) n.removeAttribute("aria-current");
  });
  row.classList.add("sel");
  const name = $(".c-name", row);
  if (name) name.setAttribute("aria-current", "true");
  follow(row.dataset.app);
  cursor = null;
  const l = list();
  go(row.dataset.href, "replace", { keep: l ? l.scrollTop : null });
}

// Moves the cursor; on the list the selected app follows after a moment.
export function step(by) {
  const c = current();
  if (!c.all.length) return;
  const i = clamp((c.i < 0 ? 0 : c.i) + by, 0, c.all.length - 1);
  const row = c.all[i];
  mark(row);
  cursor = row.dataset.app || i;
  const what = $("[data-hint]", row) || row;
  if (what.dataset.hint) hint(what.dataset.hint);
  if (row.dataset.app) follow(row.dataset.app);
  if (row.dataset.href && con().dataset.page === "list") {
    wait("select", 450, () => {
      cursor = null;
      const l = list();
      go(row.dataset.href, "replace", { keep: l ? l.scrollTop : null });
    });
  } else cancel("select", true);
}

function put(key) {
  const all = rows();
  const row = typeof key === "number" ? all[Math.min(key, all.length - 1)] : all.find((r) => r.dataset.app === key);
  if (row) mark(row);
}

// ↵ opens what the cursor is on: an app's story, a version.
export function open() {
  const c = current();
  const row = c.all[c.i];
  if (!row || !row.classList.contains("cursor") || !row.dataset.open) return false;
  go(row.dataset.open, "push");
  return true;
}

document.addEventListener("click", (e) => {
  if (!plain(e)) return;
  const row = e.target.closest && e.target.closest("#console .c-list .c-row[data-href]");
  if (!row || con().dataset.page !== "list") return;
  e.preventDefault();
  choose(row);
});

// ---- the strip beside the list: the part on screen, boxed --------------------

function frame() {
  const l = list();
  const win = $("#console .c-mini .c-win");
  if (!l || !win) return;
  const all = $$(".c-row[data-app]", l).filter((r) => !r.hidden);
  let first = 0;
  let seen = 0;
  all.forEach((r, i) => {
    const at = r.offsetTop - l.offsetTop;
    if (at + r.offsetHeight <= l.scrollTop) first = i + 1;
    else if (at < l.scrollTop + l.clientHeight) seen++;
  });
  win.setAttribute("y", String(first));
  win.setAttribute("height", String(Math.max(1, seen)));
  const range = $("#console .c-range");
  if (range && all.length) range.textContent = fill(range.dataset.of, { first: first + 1, last: Math.min(all.length, first + seen), n: all.length });
}
document.addEventListener(
  "scroll",
  (e) => {
    if (e.target === list()) frame();
  },
  { capture: true, passive: true }
);

// ---- Find: in the list's first line; the list narrows as you type ------------

// / focuses Find; from another page, the list first.
export function find() {
  const q = document.getElementById("c-q");
  if (q) {
    q.focus();
    q.select();
    return;
  }
  const to = $('#console [data-key="Escape"]');
  if (!to) return;
  go(to.getAttribute("href").replace("#console", ""), "push").then((ok) => {
    const q = ok && document.getElementById("c-q");
    if (q) q.focus();
  });
}

// What the shell's Find matches: name, id, owner or group (data-find).
export function narrow(text) {
  const q = text.trim().toLowerCase();
  $$("#console .c-list .c-row[data-find]").forEach((r) => {
    r.hidden = q !== "" && !r.dataset.find.includes(q);
  });
  $$("#console .c-list .c-sep").forEach((s) => {
    s.hidden = q !== "";
  });
  frame();
}
document.addEventListener("input", (e) => {
  if (e.target.id === "c-q") narrow(e.target.value);
});

// ---- the hint line: what the pointer or focus is on ---------------------------

document.addEventListener("pointerover", (e) => {
  const c = con();
  if (!c || !e.target.closest || !c.contains(e.target)) return;
  const el = e.target.closest("[data-hint]");
  hint(el ? el.dataset.hint : "");
});
document.addEventListener("pointerout", (e) => {
  const c = con();
  if (c && c.contains(e.target) && !c.contains(e.relatedTarget)) hint("");
});
document.addEventListener("focusin", (e) => {
  const el = e.target.closest && e.target.closest("#console [data-hint]");
  if (el) hint(el.dataset.hint);
});

// ---- the list's order, held while the pointer is on it ------------------------

document.addEventListener("pointerover", (e) => {
  const on = !!(e.target.closest && e.target.closest("#console .c-listwrap"));
  if (on === pointing) return;
  pointing = on;
  showHeld();
});
function showHeld() {
  const tag = $("#console .c-held");
  if (tag) tag.hidden = !pointing;
}

// After a refresh the rows come back in the order they had; new ones last.
function keepOrder(ids) {
  const l = list();
  if (!l) return;
  const byId = new Map($$(".c-row[data-app]", l).map((r) => [r.dataset.app, r]));
  $$(".c-sep", l).forEach((s) => s.remove());
  ids.forEach((id) => {
    const r = byId.get(id);
    if (r) {
      l.appendChild(r);
      byId.delete(id);
    }
  });
  byId.forEach((r) => l.appendChild(r));
}

onRefresh(() => {
  const l = list();
  top = l ? l.scrollTop : null;
  order = pointing && l ? $$(".c-row[data-app]", l).map((r) => r.dataset.app) : null;
});

onPage((main, how) => {
  if (!con()) return;
  if (how.refreshed && order) keepOrder(order);
  const l = list();
  if (l) {
    if (how.refreshed && top !== null) l.scrollTop = top;
    else {
      if (how.keep !== null && how.keep !== undefined) l.scrollTop = how.keep;
      reveal($(".c-row.sel", l));
    }
    frame();
  }
  if (how.refreshed && cursor !== null) put(cursor);
  else cursor = null;
  showHeld();
  top = null;
  order = null;
});


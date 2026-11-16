// What every part of the console's script looks up: the console, its
// list, the hint line, and what the shell put in data-* attributes. The
// script builds no links and no words: they come from the page.

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

export const con = () => document.getElementById("console");
export const list = () => $("#console .c-lcd .c-list");

export const clamp = (x, lo, hi) => Math.max(lo, Math.min(hi, x));

// Someone types in a field: keys are theirs.
export function typing() {
  const a = document.activeElement;
  return !!a && /^(INPUT|TEXTAREA|SELECT)$/.test(a.tagName);
}

// A plain left click, no modifier: the console's to handle; the rest are
// the browser's (a new tab, say).
export function plain(e) {
  return !(e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey);
}

// A JSON array the shell put in data-<name>; [] if there is none.
export function json(el, name) {
  try {
    const v = JSON.parse(el.dataset[name] || "[]");
    return Array.isArray(v) ? v : [];
  } catch (err) {
    return [];
  }
}

// The shell's text with {name}s filled in.
export function fill(text, values) {
  return (text || "").replace(/\{(\w+)\}/g, (m, k) => (k in values ? String(values[k]) : m));
}

// The screen's last line: `text`, or what it says by default.
export function hint(text) {
  const h = $("#console .c-hint");
  if (h) h.textContent = text || h.dataset.default || "";
}

// What finds a console control again on the next page: its knob, fader,
// pin or key, its id, or its link.
export function focusKey(el) {
  if (!el || !el.closest || !el.closest("#console")) return null;
  for (const name of ["knob", "fader", "pin", "key"]) {
    if (el.dataset[name]) return `[data-${name}="${CSS.escape(el.dataset[name])}"]`;
  }
  if (el.id) return "#" + CSS.escape(el.id);
  const href = el.getAttribute("href");
  return href ? `a[href="${CSS.escape(href)}"]` : null;
}

export function refocus(root, key) {
  if (!key) return;
  const el = root.querySelector("#console " + key);
  if (el && document.activeElement !== el) el.focus({ preventScroll: true });
}

// The keyboard, while the console is on screen: every key with a letter in
// its corner presses it, ↑↓ (j k) the cursor, ↵ opens what it is on, /
// Find, ← → the requests page's step, − + the range, 1–8 the pins. A
// focused knob or fader takes the arrows, Home and End. Held down, a key
// that presses something presses it once; only moves repeat.
import { $, $$, con, typing } from "./dom.js";
import { knob } from "./dial.js";
import { turn } from "./knobs.js";
import { slide, rest, at, last } from "./faders.js";
import { jump } from "./pins.js";
import { step, open, find, narrow } from "./screen.js";

const UP = /^(ArrowUp|ArrowRight|PageUp)$/;
const SLIDES = /^(ArrowUp|ArrowDown|ArrowLeft|ArrowRight|Home|End|PageUp|PageDown)$/;

function press(sel) {
  const el = $(sel);
  if (!el || el.closest("[inert]")) return false;
  el.classList.add("down");
  setTimeout(() => el.classList.remove("down"), 120);
  el.click();
  return true;
}

function escape(e) {
  const input = document.activeElement;
  // Typed in Find, not yet sent: Esc clears it; else Esc is List's.
  const sent = new URLSearchParams(location.search).get("q") || "";
  input.blur();
  if (input.id === "c-q" && input.value.trim() !== sent) {
    input.value = sent;
    narrow(sent);
  } else press('#console [data-key="Escape"]');
  e.preventDefault();
}

// The step key before or after the one whose requests the screen shows.
function neighbour(by) {
  const keys = $$("#console .c-step .c-key");
  const next = keys[keys.findIndex((k) => k.hasAttribute("aria-current")) + by];
  if (next) next.click();
}

document.addEventListener("keydown", (e) => {
  const c = con();
  if (!c || e.ctrlKey || e.metaKey || e.altKey) return;
  // Only while the console is on screen: further down, keys are the page's.
  const box = c.getBoundingClientRect();
  if (box.bottom < 80 || box.top > innerHeight - 80) return;
  if (typing()) {
    if (e.key === "Escape") escape(e);
    return;
  }
  const focused = document.activeElement && document.activeElement.closest ? document.activeElement : null;
  const f = focused && focused.closest("#console .c-fdr");
  if (f && SLIDES.test(e.key)) {
    slide(f, e.key === "Home" ? 0 : e.key === "End" ? last(f) : at(f) + (UP.test(e.key) ? 1 : -1));
    rest(f, false);
    e.preventDefault();
    return;
  }
  const k = focused && focused.closest("#console [data-knob]");
  if (k && SLIDES.test(e.key)) {
    turn(k, e.key === "Home" ? -Infinity : e.key === "End" ? Infinity : UP.test(e.key) ? 1 : -1);
    e.preventDefault();
    return;
  }
  const key = e.key;
  let done = true;
  if (key === "ArrowDown" || key === "j") step(1);
  else if (key === "ArrowUp" || key === "k") step(-1);
  else if (key === "-" || key === "+" || key === "=") {
    const z = knob("zoom");
    if (z) turn(z, key === "-" ? 1 : -1);
  } else if ((key === "ArrowLeft" || key === "ArrowRight") && c.dataset.page === "log") {
    if (!e.repeat) neighbour(key === "ArrowLeft" ? -1 : 1);
  } else if (e.repeat) done = key.length === 1 || key === "Enter" || key === "Escape";
  else if (key === "/") find();
  else if (/^[1-8]$/.test(key)) jump(Number(key) - 1);
  else if (key === "Enter") {
    if (focused && focused.closest("#console a, #console button")) done = false;
    else if (!open()) press('#console [data-key="Enter"]');
  } else if (key.length > 1 && key !== "Escape") done = false;
  else done = press(`#console [data-key="${CSS.escape(key.length === 1 ? key.toLowerCase() : key)}"]`);
  if (done) e.preventDefault();
});

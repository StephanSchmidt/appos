// Pins: eight keys that keep apps at hand, in this browser only. A click
// (or 1–8) jumps to the pinned app; a hold pins the selected one. A set
// pin's LED is blue; the one whose app is on the screen is pressed and
// orange. The words and the page to jump to are the shell's (data-* on
// the pins).
import { $, $$, con, fill, hint } from "./dom.js";
import { go, onPage } from "./nav.js";

const PINS = "appos.console.pins";
const HOLD_MS = 600;

function load() {
  try {
    const p = JSON.parse(localStorage.getItem(PINS) || "[]");
    return Array.isArray(p) ? p : [];
  } catch (err) {
    return [];
  }
}
function save(p) {
  try {
    localStorage.setItem(PINS, JSON.stringify(p));
  } catch (err) {
    /* private mode: pins last for the page */
  }
}

const pinned = load();
const box = () => $("#console .c-pins");

function draw() {
  const pins = box();
  if (!pins) return;
  const here = con().dataset.app;
  $$(".c-pin", pins).forEach((b, i) => {
    const p = pinned[i];
    const led = $(".c-led", b);
    led.className = "c-led";
    b.classList.remove("down");
    b.removeAttribute("aria-current");
    if (!p) return;
    b.dataset.app = p.id;
    $(".c-pinname", b).textContent = p.name;
    b.dataset.hint = fill(pins.dataset.set, { i: i + 1, name: p.name });
    const shown = p.id === here;
    b.classList.toggle("down", shown);
    led.classList.add(shown ? "orange" : "blue");
    if (shown) b.setAttribute("aria-current", "true");
  });
}
onPage(draw);

export function jump(i) {
  const p = pinned[i];
  const pins = box();
  if (p && pins) go(fill(pins.dataset.jump, { id: encodeURIComponent(p.id) }), "push");
}

// A hold: the pointer down on a pin, still, for a moment. Scrolling, a
// move or the browser taking the pointer cancels it.
let holding = null;
function drop() {
  if (holding) clearTimeout(holding.t);
  holding = null;
}
document.addEventListener("pointerdown", (e) => {
  const b = e.target.closest && e.target.closest("#console .c-pin");
  if (!b || e.button !== 0) return;
  drop();
  const i = Number(b.dataset.pin) - 1;
  const h = { i, x: e.clientX, y: e.clientY, done: false };
  h.t = setTimeout(() => {
    const c = con();
    pinned[i] = { id: c.dataset.app, name: c.dataset.name };
    save(pinned);
    h.done = true;
    draw();
    hint(fill(box().dataset.pinned, { i: i + 1, name: c.dataset.name }));
  }, HOLD_MS);
  holding = h;
});
document.addEventListener("pointerup", () => {
  if (holding) clearTimeout(holding.t);
});
document.addEventListener("pointercancel", drop);
document.addEventListener("pointermove", (e) => {
  if (holding && !holding.done && Math.hypot(e.clientX - holding.x, e.clientY - holding.y) > 8) drop();
});
document.addEventListener(
  "scroll",
  () => {
    if (holding && !holding.done) drop();
  },
  { capture: true, passive: true }
);
// A long press on a touch screen is a hold, not a menu.
document.addEventListener("contextmenu", (e) => {
  if (e.target.closest && e.target.closest("#console .c-pin")) e.preventDefault();
});
document.addEventListener("click", (e) => {
  const b = e.target.closest && e.target.closest("#console .c-pin");
  if (!b) return;
  const i = Number(b.dataset.pin) - 1;
  const wasHold = holding && holding.done && holding.i === i;
  holding = null;
  if (wasHold) return;
  if (pinned[i]) jump(i);
  else hint(b.dataset.hint);
});

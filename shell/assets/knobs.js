// Knobs turn with the wheel, a vertical drag or the arrow keys; a push (a
// click) follows the link the shell gave it. A turned knob shows its value
// at once and goes to that value's page once it stops turning. APP moves
// the cursor where the screen lists apps; a click on one of its lights
// picks that app.
import { con, hint, plain } from "./dom.js";
import { knob, stops, show, shown } from "./dial.js";
import { go, wait, settled, hold, release } from "./nav.js";
import { step, choose, rowOf } from "./screen.js";

// Turns a knob by `by` values.
export function turn(k, by) {
  const name = k.dataset.knob;
  const page = con().dataset.page;
  if (name === "app" && (page === "list" || page === "map")) {
    step(by);
    return;
  }
  const from = shown(k);
  const i = show(k, from + by);
  if (i === from) return;
  const s = stops(k);
  hint(s.hints[i]);
  const value = s.values[i];
  // Once it rests, from the page on screen then: a page that came in
  // meanwhile has links of its own.
  wait("knob:" + name, 250, () => settled().then(() => land(name, value)));
}

function land(name, value) {
  const k = knob(name);
  if (!k) return;
  const s = stops(k);
  const i = s.values.indexOf(value);
  if (i < 0 || i === Number(k.dataset.at)) return;
  go(s.hrefs[i], "replace");
}

// ---- APP's lights -------------------------------------------------------------

// The light nearest the pointer, when it is in the ring round the cap (a
// light is too small to hit), else null. Lights run from -135° to 135°,
// 0° up, as the shell draws them.
function lightAt(k, e) {
  const svg = k && k.dataset.knob === "app" && k.querySelector(".c-dots");
  if (!svg) return null;
  const box = svg.getBoundingClientRect();
  const s = box.width / 128;
  const dx = (e.clientX - box.left) / s - 64;
  const dy = (e.clientY - box.top) / s - 64;
  const d = Math.hypot(dx, dy);
  const ang = (Math.atan2(dx, -dy) * 180) / Math.PI;
  if (d < 50 || d > 72 || Math.abs(ang) > 150) return null;
  const dots = svg.querySelectorAll("circle");
  const n = dots.length;
  const i = n > 1 ? Math.round(((Math.max(-135, Math.min(135, ang)) + 135) * (n - 1)) / 270) : 0;
  return dots[i] || null;
}

// A light picks its app: on the list in place, elsewhere the same page for it.
function pick(k, i) {
  const s = stops(k);
  const id = s.values[i];
  if (id === undefined) return;
  const row = rowOf(id);
  if (row && con().dataset.page === "list") {
    choose(row);
    return;
  }
  show(k, i);
  go(s.hrefs[i], "push");
}

document.addEventListener("click", (e) => {
  if (!plain(e)) return;
  const k = e.target.closest && e.target.closest('#console [data-knob="app"]');
  const dot = lightAt(k, e);
  if (!dot) return;
  e.preventDefault();
  pick(k, Number(dot.dataset.i));
});

// Hovering the ring rings the light a click would pick and names its app.
let ringed = null;
document.addEventListener("pointermove", (e) => {
  if (drag) return;
  const k = e.target.closest && e.target.closest('#console [data-knob="app"]');
  const dot = lightAt(k, e);
  if (dot === ringed) return;
  if (ringed) ringed.classList.remove("hov");
  if (dot) dot.classList.add("hov");
  ringed = dot;
  if (k) hint(dot ? dot.dataset.hint : k.dataset.hint);
});

// ---- the wheel and a drag ------------------------------------------------------

let wheeled = 0;
document.addEventListener(
  "wheel",
  (e) => {
    const k = e.target.closest && e.target.closest("#console [data-knob]");
    if (!k) return;
    e.preventDefault();
    wheeled += e.deltaY;
    if (Math.abs(wheeled) >= 40) {
      turn(k, wheeled < 0 ? 1 : -1);
      wheeled = 0;
    }
  },
  { passive: false }
);

// A vertical drag: up turns right. Detent knobs move every 24px, the
// others every 8px. The click that ends a drag on the knob is no push.
let drag = null;
let noClick = null;
document.addEventListener("pointerdown", (e) => {
  const k = e.target.closest && e.target.closest("#console [data-knob]");
  if (!k || e.button !== 0) return;
  e.preventDefault();
  k.focus({ preventScroll: true });
  drag = { k, y: e.clientY, moved: false, each: k.classList.contains("c-det") && k.dataset.knob !== "app" ? 24 : 8 };
  k.setPointerCapture(e.pointerId);
});
document.addEventListener("pointermove", (e) => {
  if (!drag) return;
  const steps = Math.trunc((drag.y - e.clientY) / drag.each);
  if (steps === 0) return;
  if (!drag.moved) {
    drag.moved = true;
    drag.k.classList.add("turning");
    hold();
  }
  drag.y -= steps * drag.each;
  turn(drag.k, steps);
});
function endDrag() {
  if (!drag) return;
  const { k, moved } = drag;
  drag = null;
  k.classList.remove("turning");
  if (!moved) return;
  release();
  // Only that knob's click, and only right after: a touch may send none.
  noClick = { k, until: performance.now() + 400 };
}
document.addEventListener("pointerup", endDrag);
document.addEventListener("pointercancel", endDrag);
document.addEventListener("lostpointercapture", endDrag);
document.addEventListener(
  "click",
  (e) => {
    const block = noClick;
    noClick = null;
    if (block && performance.now() < block.until && block.k.contains(e.target)) {
      e.preventDefault();
      e.stopImmediatePropagation();
    }
  },
  { capture: true }
);

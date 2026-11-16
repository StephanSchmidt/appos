// Faders: an app's limits per request. Drag the cap, press the scale,
// turn the wheel, or the arrows when focused; the step a fader comes to
// rest on is posted once, by its button's form. Each fader waits on its
// own timer, so two moved at once both land.
import { $$, clamp, hint, json } from "./dom.js";
import { wait, send, hold, release } from "./nav.js";

// A fader's steps, buttons that post their value, bottom first.
function buttons(f) {
  return $$(".c-slot button", f).reverse();
}
export function at(f) {
  return Number(f.getAttribute("aria-valuenow"));
}
export function last(f) {
  return buttons(f).length - 1;
}

// Shows step i at once: the cap there, its value below.
export function slide(f, i) {
  const n = buttons(f).length;
  if (!n) return;
  i = clamp(i, 0, n - 1);
  const labels = json(f, "labels");
  f.querySelector(".c-slot").style.setProperty("--p", ((i + 0.5) * 100) / n + "%");
  const val = f.querySelector(".c-val");
  if (val) val.textContent = labels[i];
  f.setAttribute("aria-valuenow", String(i));
  f.setAttribute("aria-valuetext", labels[i]);
  hint(json(f, "hints")[i]);
}

// Once it rests, its step is set; nothing when it is back where the page
// had it (data-at).
export function rest(f, now) {
  wait(
    "fader:" + f.dataset.fader,
    now ? 0 : 450,
    () => {
      const i = at(f);
      const b = buttons(f)[i];
      if (i === Number(f.dataset.at) || !b || !b.form) return;
      send(b.form.getAttribute("action"), new URLSearchParams(new FormData(b.form, b)), b.form);
    },
    true
  );
}

function stepAt(slot, y) {
  const r = slot.getBoundingClientRect();
  return Math.floor(((r.bottom - y) / r.height) * slot.querySelectorAll("button").length);
}

// Drag the cap, or press anywhere on the scale: it goes there.
let sliding = null;
document.addEventListener("pointerdown", (e) => {
  const slot = e.target.closest && e.target.closest("#console .c-fdr .c-slot");
  if (!slot || e.button !== 0) return;
  e.preventDefault();
  sliding = { f: slot.closest(".c-fdr"), slot };
  slot.setPointerCapture(e.pointerId);
  sliding.f.classList.add("turning");
  hold();
  slide(sliding.f, stepAt(slot, e.clientY));
});
document.addEventListener("pointermove", (e) => {
  if (sliding) slide(sliding.f, stepAt(sliding.slot, e.clientY));
});
function endSlide() {
  if (!sliding) return;
  const { f } = sliding;
  sliding = null;
  f.classList.remove("turning");
  rest(f, true);
  release();
}
document.addEventListener("pointerup", endSlide);
document.addEventListener("pointercancel", endSlide);
document.addEventListener("lostpointercapture", endSlide);
// The pointer does it all; a step's button is for pages without the script.
document.addEventListener(
  "click",
  (e) => {
    if (e.target.closest && e.target.closest("#console .c-fdr .c-slot")) e.preventDefault();
  },
  true
);

let wheeled = 0;
document.addEventListener(
  "wheel",
  (e) => {
    const f = e.target.closest && e.target.closest("#console .c-fdr");
    if (!f) return;
    e.preventDefault();
    wheeled += e.deltaY;
    if (Math.abs(wheeled) >= 40) {
      slide(f, at(f) + (wheeled < 0 ? 1 : -1));
      wheeled = 0;
      rest(f, false);
    }
  },
  { passive: false }
);

// The admin screen's own script, as modules from /admin/assets/; htmx and
// its SSE extension do the live refresh. Every page works without it, just
// not live, and the console's knobs and keys are plain links and forms.
//
// This one: the header says whether the page is live ("Live" while the
// stream is open, "Reconnecting" when it dropped, "Stale" when it went
// quiet: the shell sends at least every 15 s), the tab's icon follows the
// platform's state, and a row whose status changed gets a short highlight.
//
// The console's parts:
// - nav.js: the one navigation controller: page swaps, changes, each
//   control's timer, the live refresh, focus.
// - dom.js: lookups every part shares; dial.js: a knob's face.
// - screen.js: rows, the cursor, Find, the hint line, the held order.
// - knobs.js, faders.js, pins.js: the controls; keys.js: the keyboard.
import { onPage, pageIn } from "./nav.js";
import "./screen.js";
import "./knobs.js";
import "./faders.js";
import "./pins.js";
import "./keys.js";

const root = document.documentElement;
// Parts that need the script (the pins) show from here on.
root.dataset.script = "on";

const QUIET_MS = 40000;
let last = Date.now();
const live = (state) => {
  root.dataset.live = state;
};
document.addEventListener("htmx:sseOpen", () => {
  last = Date.now();
  live("on");
});
document.addEventListener("htmx:sseError", () => live("off"));
document.addEventListener("htmx:sseMessage", () => {
  last = Date.now();
  if (root.dataset.live !== "off") live("on");
});
setInterval(() => {
  if (root.dataset.live === "on" && Date.now() - last > QUIET_MS) live("stale");
}, 5000);

// Each app's status on the console's list, by its id.
function statuses(main) {
  const out = new Map();
  main.querySelectorAll(".c-row[data-app]").forEach((el) => out.set(el.dataset.app, el.dataset.status));
  return out;
}

const main = document.getElementById("live");
let seen = main ? statuses(main) : new Map();

onPage((main) => {
  const icon = document.querySelector('link[rel="icon"]');
  if (icon && main.dataset.icon) icon.href = main.dataset.icon;
  const now = statuses(main);
  main.querySelectorAll(".c-row[data-app]").forEach((el) => {
    const before = seen.get(el.dataset.app);
    if (before !== undefined && before !== el.dataset.status) el.classList.add("changed");
  });
  seen = now;
});

// The first page, as if swapped in.
if (main) pageIn(main, { refreshed: false, keep: null });

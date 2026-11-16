// A knob's face at one of its values: the pointer, the lit dot or light,
// the ring and the value under it, as the shell drew them. Its values,
// labels, pages and hints are the shell's, in data-*; aria-valuenow is
// where it shows, data-at where the page has it.
import { $$, clamp, json } from "./dom.js";

export function knob(name) {
  return document.querySelector(`#console [data-knob="${CSS.escape(name)}"]`);
}

export function stops(k) {
  return { values: json(k, "values"), labels: json(k, "labels"), hrefs: json(k, "hrefs"), hints: json(k, "hints") };
}

export function shown(k) {
  return Number(k.getAttribute("aria-valuenow"));
}

const angle = (i, n) => -135 + (n > 1 ? (270 * i) / (n - 1) : 0) + "deg";

// Shows value i (kept in range); returns where it is.
export function show(k, i) {
  const s = stops(k);
  const n = s.values.length;
  if (!n) return shown(k);
  i = clamp(i, 0, n - 1);
  k.setAttribute("aria-valuenow", String(i));
  k.setAttribute("aria-valuetext", s.labels[i]);
  const val = k.parentNode.querySelector(".c-val");
  if (val) val.textContent = s.labels[i];
  const face = k.querySelector(".c-knob");
  const dots = $$(".c-dots circle", k);
  if (dots.length && dots[0].dataset.from !== undefined) {
    // The App knob: a light stands for the apps from its data-from on.
    let lit = 0;
    dots.forEach((c, j) => {
      if (Number(c.dataset.from) <= i) lit = j;
    });
    dots.forEach((c, j) => c.classList.toggle("on", j === lit));
    if (face) face.style.setProperty("--a", angle(lit, dots.length));
  } else {
    dots.forEach((c, j) => c.classList.toggle("on", j === i));
    if (face) face.style.setProperty("--a", angle(i, n));
    if (k.classList.contains("c-ring")) k.style.setProperty("--v", (n > 1 ? i / (n - 1) : 0) * 270 + "deg");
  }
  return i;
}

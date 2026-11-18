// Kitewell's look, the same as kitewell/templates/style.css: the platform's
// booking pages and every app of the flow share one stylesheet. Each page
// puts it into its <head> (see fullPage() in src/flow_view.ts).

export const STYLE = `
/* Kitewell's booking pages, in the look of Kitewell's pricing page
   (examples/pricing-zig). The same file is in every app of the flow
   (packages, workspace) and in the platform's own pages (kitewell/templates),
   so the customer sees one design; the shell adds only the top bar.
   Phone first; the price moves beside the form from 880px. */
:root {
  color-scheme: light;
  --ink: #171A33;
  --slate: #545873;
  --indigo: #3B3FD8;
  --indigo-deep: #2B2EA8;
  --indigo-wash: #EEEFFD;
  --indigo-hover: #E1E3FC;
  --canvas: #F7F8FC;
  --paper: #FFFFFF;
  --rule: #E2E4EF;
  --out: #F1F2F6;
  --out-ink: #7D8099;
  --save: #0B7A4B;
  --warn: #8F3A0E;
  --warn-wash: #FFF3EA;
  --radius: 14px;
  --check: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20'%3E%3Cpath d='m5 10.5 3.2 3.2L15 6.8' fill='none' stroke='%233B3FD8' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
  --check-white: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20'%3E%3Cpath d='m5 10.5 3.2 3.2L15 6.8' fill='none' stroke='%23fff' stroke-width='2.4' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
}

*, *::before, *::after { box-sizing: border-box; }

body {
  margin: 0;
  color: var(--ink);
  background: var(--canvas);
  font: 16px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  -webkit-text-size-adjust: 100%;
}

.flow {
  max-width: 1120px;
  margin: 0 auto;
  padding: 24px 16px 64px;
  display: grid;
  gap: 24px;
}
.flow h1, .flow h2, .flow p, .flow ul, .flow ol, .flow dl, .flow dd, .flow fieldset { margin: 0; }
.flow a { color: var(--indigo); text-underline-offset: 3px; }
.flow :focus-visible { outline: 3px solid var(--indigo); outline-offset: 2px; }

/* the steps of the booking: a bar per step, filled up to this one */
.steps {
  display: flex; gap: 6px;
  padding: 0; list-style: none; counter-reset: step;
  font-size: .85rem; font-weight: 600; color: var(--out-ink);
}
.steps li {
  flex: 1 1 0; min-width: 0;
  counter-increment: step;
  padding-top: 8px;
  border-top: 4px solid var(--rule);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.steps li::before { content: counter(step) ". "; }
.steps .done { border-color: var(--indigo); color: var(--slate); }
.steps .now { border-color: var(--indigo); color: var(--indigo); font-weight: 700; }
@media (max-width: 559px) {
  .steps .now { flex-grow: 3; }
  .steps li:not(.now) span { display: none; }
}

.intro { display: grid; gap: 6px; }
.intro h1 { font-size: clamp(1.75rem, 5vw, 2.5rem); line-height: 1.1; letter-spacing: -.025em; font-weight: 750; }
.lead { color: var(--slate); max-width: 62ch; }

/* the form, and the price beside it */
.split { display: grid; gap: 20px; align-items: start; }
.main { display: grid; gap: 20px; min-width: 0; }

.card {
  display: grid; gap: 14px; align-content: start;
  padding: 20px;
  background: var(--paper);
  border: 1px solid var(--rule);
  border-radius: var(--radius);
}
.card h2 { font-size: 1.15rem; line-height: 1.3; letter-spacing: -.01em; font-weight: 700; }

/* fields */
.field { display: grid; gap: 6px; }
.flow label, .group legend { font-weight: 650; font-size: .95rem; }
.flow input[type=text], .flow input[type=email], .flow input[type=number], .flow select {
  width: 100%; min-height: 48px;
  padding: 10px 14px;
  font: inherit; color: var(--ink);
  background: var(--paper);
  border: 2px solid var(--rule);
  border-radius: 10px;
  transition: border-color .15s;
}
.flow input[type=text]:focus, .flow input[type=email]:focus, .flow input[type=number]:focus, .flow select:focus {
  outline: none; border-color: var(--indigo);
}
.flow input[readonly] { background: var(--canvas); color: var(--slate); font-size: .9rem; min-height: 42px; }
.flow input[type=number] { -moz-appearance: textfield; appearance: textfield; }
.flow input[type=number]::-webkit-inner-spin-button, .flow input[type=number]::-webkit-outer-spin-button { -webkit-appearance: none; margin: 0; }
.field:has(.error:not(:empty)) input { border-color: #E8A57F; }
.row { display: flex; flex-wrap: wrap; gap: 10px; align-items: stretch; }
.flow .row > input { flex: 1 1 220px; width: auto; }

.group { border: 0; padding: 0; min-width: 0; display: grid; gap: 8px; }
.group legend { padding: 0; margin-bottom: 8px; }

.flow .choice {
  display: flex; gap: 12px; align-items: flex-start;
  font-weight: 400; cursor: pointer;
}
.choice input { flex: none; width: 20px; height: 20px; margin: 2px 0 0; accent-color: var(--indigo); }
.choice small { display: block; color: var(--slate); font-size: .9rem; }
.choice strong { font-weight: 650; }

/* buttons: indigo for the step's one action, a wash for the others */
.flow button, .flow .button {
  display: inline-flex; align-items: center; justify-content: center;
  min-height: 48px; padding: 12px 22px;
  font: inherit; font-weight: 650; line-height: 1.2; text-decoration: none;
  color: #fff; background: var(--indigo);
  border: 0; border-radius: 10px;
  cursor: pointer;
  transition: background .15s, color .15s;
}
.flow button:hover, .flow .button:hover { background: var(--indigo-deep); }
.flow .quiet { color: var(--indigo); background: var(--indigo-wash); }
.flow .quiet:hover { background: var(--indigo-hover); }

.actions { display: flex; flex-wrap: wrap; align-items: center; gap: 12px 24px; padding-top: 4px; }
.flow .actions button, .flow .actions .button { min-width: 200px; }
.flow .back { color: var(--slate); font-weight: 600; font-size: .95rem; }

.note { color: var(--slate); font-size: .9rem; }
.error {
  font-size: .92rem;
  color: var(--warn);
  background: var(--warn-wash);
  border-radius: 10px;
  padding: 10px 12px;
}
.error:empty { display: none; }
.error a { color: inherit; font-weight: 650; }

.badge {
  justify-self: start;
  font-size: .8rem; font-weight: 700;
  color: #fff; background: var(--indigo);
  padding: 4px 10px; border-radius: 999px;
  white-space: nowrap;
}

/* the team size: the number in big type, the slider a shortcut under it */
.size { gap: 8px; }
.size-field {
  display: flex; align-items: baseline; gap: 10px;
  padding: 4px 16px;
  border: 2px solid var(--rule);
  border-radius: 12px;
  background: var(--paper);
  transition: border-color .15s;
}
.size-field:focus-within { border-color: var(--indigo); }
.flow .size-field input {
  flex: 1; min-width: 0; width: 100%; min-height: 0;
  padding: 0; border: 0; outline: none; background: transparent;
  font-size: 2.75rem; font-weight: 750; line-height: 1.2;
  letter-spacing: -.03em;
  font-variant-numeric: tabular-nums;
  color: var(--indigo);
}
.size-field span { color: var(--slate); font-size: 1.05rem; }
.slider { --thumb: 24px; display: grid; gap: 2px; padding-top: 6px; }
.slider input {
  -webkit-appearance: none; appearance: none;
  width: 100%; height: var(--thumb); margin: 0;
  background: transparent; cursor: pointer;
}
.slider input::-webkit-slider-runnable-track { height: 6px; border-radius: 3px; background: var(--rule); }
.slider input::-moz-range-track { height: 6px; border-radius: 3px; background: var(--rule); }
.slider input::-moz-range-progress { height: 6px; border-radius: 3px; background: var(--indigo); }
.slider input::-webkit-slider-thumb {
  -webkit-appearance: none; box-sizing: border-box;
  width: var(--thumb); height: var(--thumb); margin-top: calc(3px - var(--thumb) / 2);
  border-radius: 50%; border: 2px solid var(--indigo); background: var(--paper);
  box-shadow: 0 1px 3px rgb(23 26 51 / .25);
}
.slider input::-moz-range-thumb {
  box-sizing: border-box;
  width: var(--thumb); height: var(--thumb);
  border-radius: 50%; border: 2px solid var(--indigo); background: var(--paper);
  box-shadow: 0 1px 3px rgb(23 26 51 / .25);
}
.slider input:active::-webkit-slider-thumb { background: var(--indigo-wash); }
.slider input:active::-moz-range-thumb { background: var(--indigo-wash); }
.marks {
  position: relative; height: 1.25rem;
  font-size: .8rem; color: var(--slate); font-variant-numeric: tabular-nums;
}
.marks span {
  position: absolute; top: 0;
  left: calc(var(--thumb) / 2 + var(--at) * (100% - var(--thumb)));
  transform: translateX(-50%);
}
/* Without JavaScript the slider can't send anything. */
@media (scripting: none) { .slider { display: none; } }

/* a choice of two or three, side by side */
.segments {
  display: grid; grid-auto-flow: column; grid-auto-columns: 1fr;
  padding: 4px; gap: 4px;
  background: var(--out);
  border-radius: 12px;
}
.flow .segments label { position: relative; display: block; cursor: pointer; font-weight: 600; font-size: 1rem; }
.segments input { position: absolute; inset: 0; opacity: 0; margin: 0; cursor: pointer; }
.segments span {
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  min-height: 52px; padding: 8px;
  border-radius: 9px;
  color: var(--slate); text-align: center; line-height: 1.25;
}
.segments small { font-size: .8rem; font-weight: 600; color: var(--slate); }
.segments small.save { color: var(--save); }
.segments input:checked + span {
  background: var(--paper); color: var(--ink);
  box-shadow: 0 1px 2px rgba(23, 26, 51, .12), 0 0 0 1px rgba(23, 26, 51, .04);
}
.segments input:focus-visible + span { outline: 3px solid var(--indigo); outline-offset: 1px; }

/* an add-on, switched on or off */
.flow .switch {
  display: flex; align-items: center; gap: 14px;
  font-weight: 400;
  cursor: pointer;
}
.switch > span { display: grid; gap: 1px; }
.switch strong { font-weight: 650; }
.switch small { color: var(--slate); font-size: .9rem; }
.switch input {
  appearance: none; -webkit-appearance: none;
  flex: none; position: relative;
  width: 50px; height: 30px; margin: 0;
  border-radius: 999px;
  background: #C9CCDC;
  cursor: pointer;
  transition: background .15s;
}
.switch input::before {
  content: ""; position: absolute; top: 3px; left: 3px;
  width: 24px; height: 24px; border-radius: 50%;
  background: var(--paper);
  box-shadow: 0 1px 3px rgba(23, 26, 51, .3);
  transition: transform .15s;
}
.switch input:checked { background: var(--indigo); }
.switch input:checked::before { transform: translateX(20px); }
.included {
  flex: none;
  width: 50px; height: 30px; border-radius: 999px;
  background: var(--indigo-wash) var(--check) center / 20px no-repeat;
}

/* the packages: rows line up across the cards */
.packages { display: grid; gap: 14px; }
.package {
  display: grid; gap: 14px; align-content: start;
  padding: 22px 20px;
  background: var(--paper);
  border: 1px solid var(--rule);
  border-radius: var(--radius);
}
.package.featured, .package.chosen {
  border: 2px solid var(--indigo);
  box-shadow: 0 18px 40px -24px rgba(59, 63, 216, .55);
}
.package-head { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
.package-head h2 { font-size: 1.3rem; letter-spacing: -.01em; }
.pitch { color: var(--slate); font-size: .95rem; }
.price { display: grid; gap: 4px; align-content: start; }
.price p:first-child { display: flex; flex-wrap: wrap; align-items: baseline; gap: 2px 8px; }
.amount {
  font-size: 2.6rem; line-height: 1.05; font-weight: 760; letter-spacing: -.035em;
  font-variant-numeric: tabular-nums;
}
.unit { color: var(--slate); font-size: .95rem; }
.saving { color: var(--save); font-weight: 650; font-size: .95rem; }
.bill {
  display: grid; gap: 2px; align-content: start;
  padding-top: 14px;
  border-top: 1px solid var(--rule);
  font-size: .95rem; color: var(--slate);
}
.bill strong { color: var(--ink); font-weight: 650; }
.flow .package button { width: 100%; }
.why {
  display: flex; gap: 8px; align-items: center;
  min-height: 48px;
  font-weight: 650; color: var(--ink); font-size: .95rem;
}
.why::before {
  content: ""; flex: none; width: 18px; height: 18px;
  background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20'%3E%3Ccircle cx='10' cy='10' r='8.5' fill='none' stroke='%23545873' stroke-width='1.6'/%3E%3Cpath d='M10 9v5M10 6.2v.1' stroke='%23545873' stroke-width='1.8' stroke-linecap='round'/%3E%3C/svg%3E") center / contain no-repeat;
}
.features {
  list-style: none; padding: 0; display: grid; gap: 8px; align-content: start;
  font-size: .95rem;
}
.features li { display: flex; gap: 10px; align-items: flex-start; }
.features li::before {
  content: ""; flex: none; width: 18px; height: 18px; margin-top: 2px;
  background: var(--check) center / contain no-repeat;
}
/* a package that doesn't fit the team: greyed out, still there to compare */
.package.off {
  background: var(--out);
  border: 1px solid transparent;
  box-shadow: none;
}
.package.off :is(h2, .pitch, .amount, .unit, .saving, .bill, .bill strong, .features) { color: var(--out-ink); }
.package.off .badge { background: var(--out-ink); }
.package.off .features li::before { opacity: .45; filter: grayscale(1); }
#cards.htmx-request, .size.htmx-request ~ #cards { opacity: .7; transition: opacity .2s .1s; }

.talk { max-width: 560px; border: 2px solid var(--indigo); }
.flow .talk .button { justify-self: start; }

/* the first project's template, as cards to pick from */
.templates { display: grid; gap: 12px; }
.flow .template {
  position: relative;
  display: grid; gap: 4px; align-content: start;
  padding: 16px 48px 16px 18px;
  background: var(--paper);
  border: 1px solid var(--rule);
  border-radius: 12px;
  font-weight: 400;
  cursor: pointer;
  transition: border-color .15s, background .15s;
}
.template input { position: absolute; inset: 0; opacity: 0; margin: 0; cursor: pointer; }
.template strong { font-weight: 650; }
.template small { color: var(--slate); font-size: .9rem; line-height: 1.45; }
.template::after {
  content: ""; position: absolute; top: 16px; right: 16px;
  width: 22px; height: 22px; border-radius: 50%;
  border: 2px solid var(--rule);
}
.template:hover { border-color: #C9CCDC; }
.template:has(input:checked) { border-color: var(--indigo); box-shadow: 0 0 0 1px var(--indigo); background: #FAFAFF; }
.template:has(input:checked)::after { border-color: var(--indigo); background: var(--indigo) var(--check-white) center / 14px no-repeat; }
.template:has(input:focus-visible) { outline: 3px solid var(--indigo); outline-offset: 2px; }

/* the price: what the booking functions calculated */
.quote { gap: 10px; }
.quote table { width: 100%; border-collapse: collapse; font-size: .95rem; }
.quote td { padding: 9px 0; border-bottom: 1px solid var(--rule); vertical-align: top; }
.quote td:last-child { text-align: right; white-space: nowrap; padding-left: 16px; font-variant-numeric: tabular-nums; }
.quote .total td { border-bottom: 0; padding-top: 14px; font-weight: 650; vertical-align: baseline; }
.quote .total td:last-child { font-size: 1.75rem; font-weight: 760; letter-spacing: -.03em; }
.problems { display: grid; gap: 4px; padding: 10px 12px 10px 30px; color: var(--warn); background: var(--warn-wash); border-radius: 10px; font-size: .92rem; }

/* review: what is booked, each with a way back to its step */
.rows { display: grid; }
.rows > div {
  display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 2px 16px;
  padding: 12px 0;
  border-bottom: 1px solid var(--rule);
}
.rows > div:first-child { padding-top: 0; }
.rows > div:last-child { border-bottom: 0; padding-bottom: 0; }
.rows dt { font-size: .85rem; font-weight: 650; color: var(--slate); }
.rows dd { grid-column: 1; }
.rows a { grid-column: 2; grid-row: 1 / span 2; align-self: center; font-weight: 600; font-size: .92rem; }

/* set up */
.ready { display: flex; gap: 16px; align-items: center; }
.ready::before {
  content: ""; flex: none; width: 48px; height: 48px; border-radius: 50%;
  background: var(--save) var(--check-white) center / 28px no-repeat;
}
.workspace-link { font-weight: 650; overflow-wrap: anywhere; }
.members { list-style: none; padding: 0; display: grid; }
.members li { display: grid; gap: 6px; padding: 12px 0; border-bottom: 1px solid var(--rule); }
.members li:first-child { padding-top: 0; }
.members .who { font-weight: 650; }
.members .who small { font-weight: 400; color: var(--slate); margin-left: 6px; }
.add { display: grid; gap: 12px; }
.add-fields { display: grid; gap: 12px; }

/* tablet and up */
@media (min-width: 640px) {
  .flow { padding: 32px 24px 72px; gap: 28px; }
  .card { padding: 24px; }
  .packages { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .templates { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .size { grid-template-columns: minmax(240px, 340px) minmax(0, 1fr); column-gap: 32px; align-items: end; }
  .size > label { grid-column: 1 / -1; }
  .add-fields { grid-template-columns: 1fr 1fr; }
}

/* the price beside the form */
@media (min-width: 880px) {
  .split { grid-template-columns: minmax(0, 1fr) 340px; gap: 24px; }
  .summary { position: sticky; top: 20px; }
}

/* the packages side by side; rows line up across cards */
@media (min-width: 1000px) {
  .packages { grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 16px; align-items: stretch; }
  .packages .package { grid-row: span 6; grid-template-rows: subgrid; row-gap: 14px; }
}

@media (prefers-reduced-motion: reduce) {
  * { transition: none !important; }
}
`;

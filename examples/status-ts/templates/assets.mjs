// Writes src/status/assets.ts from icon.svg and templates/style.css: the
// app embeds both as text. Run after editing either:
//
//   node templates/assets.mjs
import { readFileSync, writeFileSync } from "node:fs";

const app = new URL("../", import.meta.url);
const literal = (s) => "`" + s.replaceAll("\\", "\\\\").replaceAll("`", "\\`").replaceAll("${", "\\${") + "`";
const icon = readFileSync(new URL("icon.svg", app), "utf8");
const css = readFileSync(new URL("templates/style.css", app), "utf8");
writeFileSync(
  new URL("src/status/assets.ts", app),
  `// The app's design: the icon from the design team (icon.svg, drawn in the
// header too) and the style sheet (templates/style.css). \`appos build\`
// doesn't copy files into the app, so they are here as text; edit those
// two, then regenerate this file with \`node templates/assets.mjs\`.

/** icon.svg */
export const ICON = ${literal(icon)};

/** templates/style.css */
export const STYLE = ${literal(css)};
`,
);

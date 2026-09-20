// Builds `dist/preview.html` from the real `dist/index.html`, so the page that is looked at is
// the page that ships — a hand-copied one drifts, and a missing element takes the whole script
// down with it.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const markup = readFileSync(join(here, "../dist/index.html"), "utf8")
  .replace("<title>Orochi</title>", "<title>Orochi — preview</title>")
  .replace(
    '<script type="module" src="app.js"></script>',
    '<script src="preview-data.js"></script>\n<script type="module" src="app.js"></script>',
  );
writeFileSync(join(here, "../dist/preview.html"), markup);
console.log("preview.html built from index.html");

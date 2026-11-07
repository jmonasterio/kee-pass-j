// Bundles web/src into web/dist for Tauri to embed in the executable.
// Fails the build when any source writes HTML strings into the DOM: vault data must only
// ever reach the page through textContent / properties (see docs/ARCHITECTURE.md, UI security).
import { build } from "esbuild";
import { cpSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const src = join(root, "src");
const dist = join(root, "dist");

const banned = /\b(innerHTML|outerHTML|insertAdjacentHTML|document\.write|createContextualFragment)\b|\beval\s*\(|new\s+Function\s*\(/;
const offenders = [];
(function scan(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      scan(path);
      continue;
    }
    readFileSync(path, "utf8")
      .split("\n")
      .forEach((line, i) => {
        if (banned.test(line)) offenders.push(`${path}:${i + 1}: ${line.trim()}`);
      });
  }
})(src);
if (offenders.length) {
  console.error("HTML-string DOM APIs are not allowed in web/src:\n" + offenders.join("\n"));
  process.exit(1);
}

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });
await build({
  entryPoints: [join(src, "main.ts")],
  bundle: true,
  format: "esm",
  target: "es2022",
  minify: true,
  sourcemap: false,
  legalComments: "none",
  outfile: join(dist, "app.js"),
  logLevel: "warning",
});
cpSync(join(root, "index.html"), join(dist, "index.html"));
cpSync(join(src, "styles.css"), join(dist, "styles.css"));
console.log("web/dist built");

// Writes the updater feed (latest.json) for a tagged release. The app reads it from
// releases/latest/download/latest.json (plugins.updater.endpoints in tauri.conf.json).
//
//   node scripts/update-feed.mjs <tag> <repository> <dist dir> > latest.json
//
// <dist dir> holds the downloaded CI artifacts: one folder per artifact, each package next to
// its .sig file from the updater key.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

/** Platform key (Tauri's "<os>-<arch>") → package path inside the dist dir. The universal
 *  macOS archive serves both architectures. */
export const PACKAGES = {
  "darwin-aarch64": "OurLogins-macos-universal/OurLogins-macos-universal.app.tar.gz",
  "darwin-x86_64": "OurLogins-macos-universal/OurLogins-macos-universal.app.tar.gz",
  "windows-x86_64": "OurLogins-windows-x64/OurLogins-windows-x64-setup.exe",
  "linux-x86_64": "OurLogins-linux-x86_64/OurLogins-linux-x86_64.AppImage",
};

export function updateFeed(tag, repository, dist, now = new Date()) {
  if (!/^v\d+\.\d+\.\d+/.test(tag)) throw new Error(`not a version tag: ${tag}`);
  const base = `https://github.com/${repository}/releases/download/${tag}`;
  const platforms = {};
  for (const [platform, path] of Object.entries(PACKAGES)) {
    platforms[platform] = {
      url: `${base}/${path.split("/").pop()}`,
      signature: readFileSync(join(dist, `${path}.sig`), "utf8").trim(),
    };
  }
  return { version: tag.slice(1), pub_date: now.toISOString().replace(/\.\d+Z$/, "Z"), notes: "", platforms };
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [tag, repository, dist] = process.argv.slice(2);
  process.stdout.write(JSON.stringify(updateFeed(tag, repository, dist), null, 2) + "\n");
}

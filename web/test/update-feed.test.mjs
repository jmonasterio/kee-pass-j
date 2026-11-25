// Tests for scripts/update-feed.mjs, which writes the updater feed in the release job.
// Run: npm test

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { PACKAGES, updateFeed } from "../../scripts/update-feed.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");

function distWithSignatures() {
  const dist = mkdtempSync(join(tmpdir(), "feed-"));
  for (const path of new Set(Object.values(PACKAGES))) {
    mkdirSync(join(dist, dirname(path)), { recursive: true });
    // tauri writes the signature without a newline; a trailing one must not reach the feed.
    writeFileSync(join(dist, `${path}.sig`), `sig-of-${path.split("/").pop()}\n`);
  }
  return dist;
}

test("feed covers the platforms the app asks for, with tagged URLs and signatures", () => {
  const dist = distWithSignatures();
  try {
    const feed = updateFeed("v0.5.0", "jmonasterio/kee-pass-j", dist, new Date("2026-11-25T10:00:00.123Z"));
    assert.equal(feed.version, "0.5.0");
    assert.equal(feed.pub_date, "2026-11-25T10:00:00Z");
    // tauri-plugin-updater looks up "<os>-<arch>"; a missing key means that platform never updates.
    assert.deepEqual(Object.keys(feed.platforms).sort(), ["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"]);
    assert.deepEqual(feed.platforms["windows-x86_64"], {
      url: "https://github.com/jmonasterio/kee-pass-j/releases/download/v0.5.0/OurLogins-windows-x64-setup.exe",
      signature: "sig-of-OurLogins-windows-x64-setup.exe",
    });
    assert.equal(feed.platforms["darwin-aarch64"].url, feed.platforms["darwin-x86_64"].url);
  } finally {
    rmSync(dist, { recursive: true });
  }
});

test("a missing signature fails the release instead of publishing an unusable feed", () => {
  const dist = distWithSignatures();
  try {
    rmSync(join(dist, `${PACKAGES["linux-x86_64"]}.sig`));
    assert.throws(() => updateFeed("v0.5.0", "o/r", dist), /ENOENT/);
    assert.throws(() => updateFeed("main", "o/r", dist), /not a version tag/);
  } finally {
    rmSync(dist, { recursive: true });
  }
});

test("every package the feed points at is uploaded by the release job", () => {
  const workflow = readFileSync(join(root, ".github/workflows/build.yml"), "utf8");
  for (const path of Object.values(PACKAGES)) assert.ok(workflow.includes(`dist/${path}\n`), `release job does not upload ${path}`);
  assert.ok(workflow.includes("dist/latest.json\n"), "release job does not upload latest.json");
});

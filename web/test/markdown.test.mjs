// Tests for web/src/markdown.ts. The module is bundled with the project's esbuild into a temp
// dir and run against a small fake DOM, so no extra packages are needed.
// Run: npm test

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { build } from "esbuild";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";

class FakeText {
  constructor(data) { this.nodeType = 3; this.data = String(data); }
}

class FakeElement {
  constructor(tag) {
    this.nodeType = 1;
    this.tagName = tag.toLowerCase();
    this.attributes = new Map();
    this.childNodes = [];
    this.listeners = {};
    this.dataset = {};
    this.style = { setProperty() {} };
  }
  set className(v) { this.attributes.set("class", String(v)); }
  get className() { return this.attributes.get("class") ?? ""; }
  set href(v) { this.attributes.set("href", String(v)); }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  getAttribute(name) { return this.attributes.has(name) ? this.attributes.get(name) : null; }
  appendChild(node) { this.childNodes.push(node); return node; }
  append(...nodes) {
    for (const n of nodes) this.appendChild(typeof n === "string" ? new FakeText(n) : n);
  }
  addEventListener(name, fn) { (this.listeners[name] ??= []).push(fn); }
  click() {
    const event = { defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } };
    for (const fn of this.listeners.click ?? []) fn(event);
    return event;
  }
  get textContent() {
    return this.childNodes.map((c) => (c.nodeType === 3 ? c.data : c.textContent)).join("");
  }
  *walk() {
    for (const c of this.childNodes) if (c.nodeType === 1) { yield c; yield* c.walk(); }
  }
  find(tag) { return [...this.walk()].filter((e) => e.tagName === tag); }
  /** Serialised shape, text unescaped, for structure assertions only. */
  get shape() {
    return this.childNodes
      .map((c) => (c.nodeType === 3 ? c.data : `<${c.tagName}>${c.shape}</${c.tagName}>`))
      .join("");
  }
}

globalThis.document = {
  createElement: (tag) => new FakeElement(tag),
  createTextNode: (text) => new FakeText(text),
};

const ALLOWED_TAGS = new Set([
  "p", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "blockquote", "pre", "code",
  "strong", "em", "s", "table", "thead", "tbody", "tr", "th", "td", "a", "br", "hr",
]);

let renderMarkdown;
let outDir;

before(async () => {
  outDir = mkdtempSync(join(tmpdir(), "ourlogins-md-test-"));
  const outfile = join(outDir, "markdown.mjs");
  await build({
    entryPoints: [fileURLToPath(new URL("../src/markdown.ts", import.meta.url))],
    bundle: true,
    format: "esm",
    platform: "neutral",
    mainFields: ["module", "main"],
    outfile,
    logLevel: "silent",
  });
  ({ renderMarkdown } = await import(pathToFileURL(outfile).href));
});

after(() => { if (outDir) rmSync(outDir, { recursive: true, force: true }); });

function render(source) {
  const opened = [];
  const root = new FakeElement("div");
  renderMarkdown(source, root, (url) => opened.push(url));
  return { root, opened };
}

/** Every element is from the fixed set and carries only class/href/title attributes. */
function assertOnlySafeNodes(root) {
  for (const e of root.walk()) {
    assert.ok(ALLOWED_TAGS.has(e.tagName), `unexpected element <${e.tagName}>`);
    for (const name of e.attributes.keys()) {
      assert.ok(["class", "href", "title"].includes(name), `unexpected attribute ${name} on <${e.tagName}>`);
    }
    if (e.tagName === "a") assert.equal(e.getAttribute("href"), "#");
  }
}

test("raw <script> in vault text stays text and creates no element", () => {
  const { root } = render("hi <script>alert(1)</script> there\n\n<script>alert(2)</script>");
  assertOnlySafeNodes(root);
  assert.match(root.textContent, /<script>alert\(1\)<\/script>/);
  assert.match(root.textContent, /<script>alert\(2\)<\/script>/);
});

test("raw <img onerror> and <iframe> stay text", () => {
  const { root } = render('<img src=x onerror="alert(1)">\n\ntext <iframe src="https://evil.example"></iframe>');
  assertOnlySafeNodes(root);
  assert.match(root.textContent, /<img src=x onerror="alert\(1\)">/);
  assert.match(root.textContent, /<iframe src="https:\/\/evil.example"><\/iframe>/);
});

for (const url of [
  "javascript:alert(1)",
  "JaVaScRiPt:alert(1)",
  "data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==",
  "vbscript:msgbox(1)",
  "file:///C:/Windows/System32/calc.exe",
]) {
  test(`link to ${url.split(":")[0]}: is never opened`, () => {
    const { root, opened } = render(`[click](${url}) and <${url}>`);
    assertOnlySafeNodes(root);
    for (const a of root.find("a")) {
      assert.equal(a.getAttribute("title"), null);
      const event = a.click();
      assert.ok(event.defaultPrevented, "click must not follow href");
    }
    assert.deepEqual(opened, []);
  });
}

test("images are not loaded: dangerous and normal sources show alt text only", () => {
  const { root, opened } = render("![pic](javascript:alert(1)) ![x](data:image/png;base64,AA==) ![logo](https://e.example/a.png) ![](https://e.example/b.png)");
  assertOnlySafeNodes(root);
  assert.equal(root.find("img").length, 0);
  // markdown-it refuses javascript: as a link target, so that image stays literal text.
  assert.equal(root.textContent, "![pic](javascript:alert(1)) [x] [logo] [image]");
  assert.deepEqual(opened, []);
});

test("https, http and mailto links open through openUrl, with the URL as title", () => {
  const { root, opened } = render("[site](https://example.com/a?b=1) <http://x.example> [me](mailto:a@b.example)");
  assertOnlySafeNodes(root);
  const links = root.find("a");
  assert.equal(links.length, 3);
  assert.deepEqual(links.map((a) => a.getAttribute("class")), ["link", "link", "link"]);
  assert.deepEqual(links.map((a) => a.getAttribute("title")),
    ["https://example.com/a?b=1", "http://x.example", "mailto:a@b.example"]);
  for (const a of links) assert.ok(a.click().defaultPrevented);
  assert.deepEqual(opened, ["https://example.com/a?b=1", "http://x.example", "mailto:a@b.example"]);
  assert.equal(links[0].textContent, "site");
});

test("bare URLs are linkified", () => {
  const { root, opened } = render("see https://example.org/page now");
  const [a] = root.find("a");
  assert.equal(a.textContent, "https://example.org/page");
  a.click();
  assert.deepEqual(opened, ["https://example.org/page"]);
});

test("basic markdown renders: headings, bold, em, strike, code, lists", () => {
  const { root } = render("# Title\n\n**bold** *em* ~~gone~~ `x<y>`\n\n- one\n- two\n\n1. a\n2. b");
  assertOnlySafeNodes(root);
  assert.equal(root.shape,
    "<h1>Title</h1><p><strong>bold</strong> <em>em</em> <s>gone</s> <code>x<y></code></p>" +
    "<ul><li>one</li><li>two</li></ul><ol><li>a</li><li>b</li></ol>");
});

test("single newlines inside a paragraph become <br>", () => {
  const { root } = render("line one\nline two");
  assert.equal(root.shape, "<p>line one<br></br>line two</p>");
});

test("fenced code keeps content verbatim as text", () => {
  const { root } = render("```html\n<script>alert(1)</script>\n```");
  assertOnlySafeNodes(root);
  assert.equal(root.shape, "<pre><code><script>alert(1)</script>\n</code></pre>");
  assert.equal(root.find("script").length, 0);
});

test("tables, blockquote and hr render", () => {
  const { root } = render("> quoted\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |");
  assertOnlySafeNodes(root);
  assert.equal(root.shape,
    "<blockquote><p>quoted</p></blockquote><hr></hr>" +
    "<table><thead><tr><th>a</th><th>b</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>");
});

test("loose list items keep their paragraphs; nested tight lists stay balanced", () => {
  const { root } = render("- a\n  - b\n- c\n\nafter");
  assert.equal(root.shape, "<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul><p>after</p>");
});

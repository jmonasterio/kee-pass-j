// Markdown for document text, rendered straight to DOM nodes. markdown-it only parses (raw
// HTML off); this file turns its tokens into a fixed set of elements with text nodes, so text
// from the vault is never parsed as HTML. Line breaks inside a paragraph are kept, so plain
// text pasted from anywhere reads the same as it was typed.

import MarkdownIt, { type Token } from "markdown-it";
import { el } from "./dom";

const md = new MarkdownIt({ html: false, linkify: true, breaks: true, typographer: false });

/** Block and inline tags markdown-it can open; anything else is ignored (its text still shows). */
const TAGS: Record<string, true> = {
  p: true, h1: true, h2: true, h3: true, h4: true, h5: true, h6: true,
  ul: true, ol: true, li: true, blockquote: true, pre: true, code: true,
  strong: true, em: true, s: true, table: true, thead: true, tbody: true,
  tr: true, th: true, td: true, a: true,
};

function safeHref(href: string | null): string | null {
  if (!href) return null;
  return /^(https?:|mailto:)/i.test(href.trim()) ? href.trim() : null;
}

function renderInline(tokens: Token[], parent: HTMLElement, openUrl: (url: string) => void): void {
  const stack: HTMLElement[] = [parent];
  const top = () => stack[stack.length - 1];
  for (const t of tokens) {
    switch (t.type) {
      case "text":
        top().append(t.content);
        break;
      case "softbreak":
      case "hardbreak":
        top().append(el("br"));
        break;
      case "code_inline":
        top().append(el("code", null, t.content));
        break;
      case "image":
        // No images from documents: show the alt text instead of fetching anything.
        top().append(t.content ? `[${t.content}]` : "[image]");
        break;
      case "link_open": {
        const raw = t.attrGet("href");
        const href = safeHref(raw === null ? null : String(raw));
        const a = el("a", {
          class: "link",
          href: "#",
          attrs: { title: href ?? undefined },
          on: {
            click: (event: MouseEvent) => {
              event.preventDefault();
              if (href) openUrl(href);
            },
          },
        });
        top().append(a);
        stack.push(a);
        break;
      }
      default:
        if (t.nesting === 1 && TAGS[t.tag]) {
          const node = el(t.tag);
          top().append(node);
          stack.push(node);
        } else if (t.nesting === -1 && stack.length > 1 && (TAGS[t.tag] || t.type === "link_close")) {
          stack.pop();
        }
    }
  }
}

/** Renders Markdown (CommonMark plus tables, strikethrough and bare links) into `container`. */
export function renderMarkdown(source: string, container: HTMLElement, openUrl: (url: string) => void): void {
  const tokens = md.parse(source, {});
  const stack: HTMLElement[] = [container];
  const top = () => stack[stack.length - 1];
  for (const t of tokens) {
    if (t.hidden) {
      // Paragraphs inside "tight" lists are hidden: no element, but keep the stack balanced.
      if (t.nesting === 1) stack.push(top());
      else if (t.nesting === -1 && stack.length > 1) stack.pop();
      continue;
    }
    if (t.type === "inline") {
      renderInline(t.children ?? [], top(), openUrl);
    } else if (t.type === "fence" || t.type === "code_block") {
      top().append(el("pre", null, el("code", null, t.content)));
    } else if (t.type === "hr") {
      top().append(el("hr"));
    } else if (t.nesting === 1 && TAGS[t.tag]) {
      const node = el(t.tag);
      top().append(node);
      stack.push(node);
    } else if (t.nesting === -1 && TAGS[t.tag] && stack.length > 1) {
      stack.pop();
    }
  }
}

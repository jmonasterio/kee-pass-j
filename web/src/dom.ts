// DOM construction helpers. Text always enters the page as text nodes or via properties;
// vault data is untrusted and is never parsed as markup.

export type Child = Node | string | number | null | undefined | false | Child[];

// Method-signature form so handlers may declare a narrower event type (KeyboardEvent, ...).
type Handler = { handle(event: Event): void }["handle"];

export interface Props {
  class?: string;
  /** Attributes: `true` sets an empty attribute, `false`/null/undefined skip it. */
  attrs?: Record<string, string | number | boolean | null | undefined>;
  dataset?: Record<string, string>;
  style?: Record<string, string>;
  on?: Record<string, Handler>;
  /** Any other key is assigned as a DOM property (value, disabled, appearance, ...). */
  [prop: string]: unknown;
}

function appendChildren(parent: Node, children: Child[]): void {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    if (Array.isArray(child)) appendChildren(parent, child);
    else if (typeof child === "string" || typeof child === "number") parent.appendChild(document.createTextNode(String(child)));
    else parent.appendChild(child);
  }
}

export function el<K extends keyof HTMLElementTagNameMap>(tag: K, props?: Props | null, ...children: Child[]): HTMLElementTagNameMap[K];
export function el(tag: string, props?: Props | null, ...children: Child[]): HTMLElement;
export function el(tag: string, props?: Props | null, ...children: Child[]): HTMLElement {
  const node = document.createElement(tag);
  if (props) {
    for (const [key, value] of Object.entries(props)) {
      if (value === undefined) continue;
      switch (key) {
        case "class":
          node.className = value as string;
          break;
        case "attrs":
          for (const [name, attr] of Object.entries(value as NonNullable<Props["attrs"]>)) {
            if (attr === true) node.setAttribute(name, "");
            else if (attr !== false && attr !== null && attr !== undefined) node.setAttribute(name, String(attr));
          }
          break;
        case "dataset":
          Object.assign(node.dataset, value);
          break;
        case "style":
          for (const [name, css] of Object.entries(value as Record<string, string>)) node.style.setProperty(name, css);
          break;
        case "on":
          for (const [name, handler] of Object.entries(value as Record<string, Handler>)) node.addEventListener(name, handler);
          break;
        default:
          (node as unknown as Record<string, unknown>)[key] = value;
      }
    }
  }
  appendChildren(node, children);
  return node;
}

/** Replaces all children of `parent`. */
export function setChildren(parent: Element, ...children: Child[]): void {
  const fragment = document.createDocumentFragment();
  appendChildren(fragment, children);
  parent.replaceChildren(fragment);
}

const SVG_NS = "http://www.w3.org/2000/svg";

type Shape = ["path", string] | ["circle", number, number, number] | ["rect", number, number, number, number, number];

// 20px outline icons drawn with 1.5px strokes, in the style of Fluent System Icons (regular).
const ICONS: Record<string, Shape[]> = {
  add: [["path", "M10 4v12M4 10h12"]],
  lock: [
    ["rect", 4.75, 8.75, 10.5, 8.5, 2],
    ["path", "M7 8.75V6.5a3 3 0 0 1 6 0v2.25"],
  ],
  copy: [
    ["rect", 7, 6, 9, 11.25, 2],
    ["path", "M4.75 13.5V5a2.25 2.25 0 0 1 2.25-2.25h6.25"],
  ],
  eye: [
    ["path", "M2.5 10.5S5 5.25 10 5.25s7.5 5.25 7.5 5.25"],
    ["circle", 10, 11.25, 2.75],
  ],
  eyeOff: [
    ["path", "M2.5 10.5S5 5.25 10 5.25s7.5 5.25 7.5 5.25"],
    ["circle", 10, 11.25, 2.75],
    ["path", "M3 3l14 14"],
  ],
  open: [
    ["path", "M11.5 3.75h4.75V8.5M16 4l-6.5 6.5"],
    ["path", "M14.25 11.5v3a1.75 1.75 0 0 1-1.75 1.75h-7a1.75 1.75 0 0 1-1.75-1.75v-7A1.75 1.75 0 0 1 5.5 5.75h3"],
  ],
  settings: [
    ["circle", 10, 10, 2.5],
    [
      "path",
      "M8.6 2.9h2.8l.45 2.05 1.55.9 2-.65 1.4 2.4-1.55 1.4v1.8l1.55 1.4-1.4 2.4-2-.65-1.55.9-.45 2.05H8.6l-.45-2.05-1.55-.9-2 .65-1.4-2.4L4.75 10.9v-1.8L3.2 7.6l1.4-2.4 2 .65 1.55-.9z",
    ],
  ],
  more: [
    ["circle", 4.5, 10, 0.6],
    ["circle", 10, 10, 0.6],
    ["circle", 15.5, 10, 0.6],
  ],
  back: [["path", "M11.75 4.5 6.25 10l5.5 5.5"]],
  search: [
    ["circle", 8.5, 8.5, 5],
    ["path", "M12.25 12.25 16.5 16.5"],
  ],
  refresh: [["path", "M15.5 10a5.5 5.5 0 1 1-1.6-3.9M14.25 3.5v3h-3"]],
  dismiss: [["path", "M5 5l10 10M15 5 5 15"]],
  delete: [
    ["path", "M4 5.5h12"],
    ["path", "M8 5.5V4.25A1.25 1.25 0 0 1 9.25 3h1.5A1.25 1.25 0 0 1 12 4.25V5.5"],
    ["path", "M5.5 5.5l.7 9.6a1.75 1.75 0 0 0 1.75 1.65h4.1a1.75 1.75 0 0 0 1.75-1.65l.7-9.6"],
  ],
  folder: [["path", "M2.75 6A1.75 1.75 0 0 1 4.5 4.25h3l1.75 1.75h6.25a1.75 1.75 0 0 1 1.75 1.75v6.75A1.75 1.75 0 0 1 15.5 16.25h-11a1.75 1.75 0 0 1-1.75-1.75z"]],
  keyboard: [
    ["rect", 2.75, 5.25, 14.5, 9.5, 2],
    ["path", "M5.75 8.5h.01M8.5 8.5h.01M11.5 8.5h.01M14.25 8.5h.01M6.5 11.75h7"],
  ],
  key: [
    ["circle", 6.75, 10, 3.25],
    ["path", "M10 10h7M14.75 10v2.5M17 10v2"],
  ],
  text: [["path", "M4.5 5.5h11M4.5 10h11M4.5 14.5h7"]],
  warning: [["path", "M10 3.25 17.25 16H2.75z"], ["path", "M10 8v3.5M10 13.75h.01"]],
  document: [
    ["path", "M5.75 2.75h5.5l3.5 3.5v10a1 1 0 0 1-1 1h-8a1 1 0 0 1-1-1v-12.5a1 1 0 0 1 1-1z"],
    ["path", "M11 2.75v3.75h3.75M7.75 10.5h4.5M7.75 13.5h4.5"],
  ],
  info: [["circle", 10, 10, 7.25], ["path", "M10 9v4.5M10 6.5h.01"]],
};

export type IconName = keyof typeof ICONS;

export function icon(name: IconName, slot?: string): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 20 20");
  svg.setAttribute("width", "20");
  svg.setAttribute("height", "20");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.5");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  svg.classList.add("icon");
  if (slot) svg.setAttribute("slot", slot);
  for (const shape of ICONS[name]) {
    const node = document.createElementNS(SVG_NS, shape[0]);
    if (shape[0] === "path") node.setAttribute("d", shape[1]);
    else if (shape[0] === "circle") {
      node.setAttribute("cx", String(shape[1]));
      node.setAttribute("cy", String(shape[2]));
      node.setAttribute("r", String(shape[3]));
    } else {
      node.setAttribute("x", String(shape[1]));
      node.setAttribute("y", String(shape[2]));
      node.setAttribute("width", String(shape[3]));
      node.setAttribute("height", String(shape[4]));
      node.setAttribute("rx", String(shape[5]));
    }
    svg.appendChild(node);
  }
  return svg;
}

/** Fluent button helper. `appearance` defaults to the standard (secondary) look. */
export function button(
  label: Child,
  onClick: (event: MouseEvent) => void,
  opts: { appearance?: "primary" | "outline" | "subtle" | "transparent"; size?: "small" | "medium" | "large"; icon?: IconName; title?: string; disabled?: boolean; class?: string } = {},
): HTMLElementTagNameMap["fluent-button"] {
  const iconOnly = opts.icon !== undefined && (label === null || label === "");
  return el(
    "fluent-button",
    {
      class: opts.class,
      appearance: opts.appearance,
      size: opts.size,
      disabled: opts.disabled ?? false,
      attrs: { "icon-only": iconOnly, title: opts.title, "aria-label": iconOnly ? opts.title : undefined },
      on: { click: onClick },
    },
    opts.icon ? icon(opts.icon, iconOnly ? undefined : "start") : null,
    iconOnly ? null : label,
  );
}

/** Edge and WebView2 draw their own reveal eye inside password boxes. Ours replaces it so
 *  every password box has one button, on macOS too. Fluent keeps the <input> in its shadow
 *  root, out of reach of the page's CSS, so the rule goes into that root. */
let nativeRevealHidden: CSSStyleSheet | undefined;

/** Adds a show/hide button at the end of a Fluent text input used for secrets. */
export function addRevealToggle(input: HTMLElementTagNameMap["fluent-text-input"]): HTMLElementTagNameMap["fluent-button"] {
  const root = input.shadowRoot;
  if (root) {
    if (!nativeRevealHidden) {
      nativeRevealHidden = new CSSStyleSheet();
      nativeRevealHidden.replaceSync("input::-ms-reveal { display: none; }");
    }
    root.adoptedStyleSheets = [...root.adoptedStyleSheets, nativeRevealHidden];
  }
  const toggle: HTMLElementTagNameMap["fluent-button"] = button(
    null,
    () => {
      const show = input.type === "password";
      input.type = show ? "text" : "password";
      setChildren(toggle, icon(show ? "eyeOff" : "eye"));
      toggle.title = show ? "Hide" : "Show";
      toggle.setAttribute("aria-label", toggle.title);
      input.focus();
    },
    {
      icon: input.type === "password" ? "eye" : "eyeOff",
      appearance: "transparent",
      size: "small",
      title: input.type === "password" ? "Show" : "Hide",
      class: "reveal-toggle",
    },
  );
  toggle.setAttribute("slot", "end");
  // Clicking the button must not take focus away from the text.
  toggle.addEventListener("mousedown", (event) => event.preventDefault());
  input.append(toggle);
  return toggle;
}

/** Password box with a show/hide button at its end. */
export function secretInput(placeholder: string): HTMLElementTagNameMap["fluent-text-input"] {
  const input = el("fluent-text-input", {
    type: "password",
    placeholder,
    autocomplete: "off",
    attrs: { spellcheck: "false", autocomplete: "off" },
  });
  addRevealToggle(input);
  return input;
}

/** Labelled form field wrapper around a single Fluent input. */
export function field(label: string, input: HTMLElement, opts: { position?: "above" | "after" | "before"; class?: string } = {}): HTMLElementTagNameMap["fluent-field"] {
  input.setAttribute("slot", "input");
  return el(
    "fluent-field",
    { class: opts.class, attrs: { "label-position": opts.position ?? "above" } },
    el("label", { attrs: { slot: "label" } }, label),
    input,
  );
}

export function debounce<A extends unknown[]>(fn: (...args: A) => void, ms: number): (...args: A) => void {
  let timer: number | undefined;
  return (...args: A) => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => fn(...args), ms);
  };
}

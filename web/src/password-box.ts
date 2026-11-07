// The one input control for secrets: master passwords, account passwords, PINs and protected
// custom fields. It always uses the monospace password font, shown or hidden, so l/1/I and
// O/0 can be told apart, and it has its own show/hide button.

import { button, el, icon, setChildren } from "./dom";

type TextInputEl = HTMLElementTagNameMap["fluent-text-input"];
type ButtonEl = HTMLElementTagNameMap["fluent-button"];

/** Edge and WebView2 draw their own reveal eye inside password inputs; ours replaces it so
 *  there is one button, on macOS too. Fluent keeps the <input> in its shadow root, out of
 *  reach of the page's CSS, so the rule goes into that root. */
let nativeRevealHidden: CSSStyleSheet | undefined;

const toggles = new WeakMap<TextInputEl, ButtonEl>();

function showToggleState(toggle: ButtonEl, shown: boolean): void {
  setChildren(toggle, icon(shown ? "eyeOff" : "eye"));
  toggle.title = shown ? "Hide" : "Show";
  toggle.setAttribute("aria-label", toggle.title);
}

/**
 * A password box. `shown` starts it in clear text (the generated password in the account
 * form); `large` is the bigger font used there.
 */
export function passwordBox(opts: { value?: string; placeholder?: string; shown?: boolean; large?: boolean } = {}): TextInputEl {
  const input = el("fluent-text-input", {
    class: "password-box" + (opts.large ? " large" : ""),
    type: opts.shown ? "text" : "password",
    value: opts.value ?? "",
    placeholder: opts.placeholder ?? "",
    autocomplete: "off",
    attrs: { spellcheck: "false", autocomplete: "off" },
  });
  const root = input.shadowRoot;
  if (root) {
    if (!nativeRevealHidden) {
      nativeRevealHidden = new CSSStyleSheet();
      nativeRevealHidden.replaceSync("input::-ms-reveal { display: none; }");
    }
    root.adoptedStyleSheets = [...root.adoptedStyleSheets, nativeRevealHidden];
  }
  const toggle: ButtonEl = button(
    null,
    () => {
      const show = input.type === "password";
      input.type = show ? "text" : "password";
      showToggleState(toggle, show);
      input.focus();
    },
    { icon: "eye", appearance: "transparent", size: "small", class: "reveal-toggle" },
  );
  showToggleState(toggle, !!opts.shown);
  toggle.setAttribute("slot", "end");
  // Clicking the button must not take focus away from the text.
  toggle.addEventListener("mousedown", (event) => event.preventDefault());
  input.append(toggle);
  toggles.set(input, toggle);
  return input;
}

/** Turns a password box into a plain text field and back (a custom field's "Protected" box). */
export function setSecret(input: TextInputEl, secret: boolean): void {
  input.type = secret ? "password" : "text";
  input.classList.toggle("password-box", secret);
  const toggle = toggles.get(input);
  if (toggle) {
    toggle.hidden = !secret;
    showToggleState(toggle, false);
  }
}

import { api, errorText, onAutotypeChoose, onToast, type PlatformInfo } from "./api";
import { isOverlayOpen } from "./dialogs";
import { el, setChildren } from "./dom";
import { registerFluent } from "./fluent";
import { nav } from "./nav";
import { initTheme } from "./theme";
import { toast } from "./toast";
import { openChooser } from "./views/chooser";
import { mainKeydown, renderMain } from "./views/main";
import { renderUnlock, unlockKeydown } from "./views/unlock";

type Page = "unlock" | "main";

async function start(): Promise<void> {
  const root = document.getElementById("app")!;
  let page: Page = "unlock";

  await registerFluent();

  let info: PlatformInfo;
  try {
    info = await api.platformInfo();
  } catch {
    // Without the backend answer, fall back to an opaque window and the user agent's OS.
    info = { os: /Mac/.test(navigator.userAgent) ? "macos" : /Win/.test(navigator.userAgent) ? "windows" : "linux", backdrop: false, notice: null };
  }
  document.documentElement.dataset.os = info.os;
  document.documentElement.dataset.backdrop = String(info.backdrop);
  initTheme(info.os);

  nav.unlock = (view) => {
    page = "unlock";
    renderUnlock(root, view);
  };
  nav.main = (view) => {
    page = "main";
    renderMain(root, view);
  };

  document.addEventListener("keydown", (event) => {
    if (isOverlayOpen()) return;
    if (page === "unlock") unlockKeydown(event);
    else mainKeydown(event);
  });

  // The webview's own context menu (Back, Reload, Print, Inspect) does not belong in a desktop
  // app. Text fields keep it for cut/copy/paste.
  document.addEventListener("contextmenu", (event) => {
    const editable = event.composedPath().some(
      (node) => node instanceof HTMLElement && (node.matches("input, textarea") || node.isContentEditable),
    );
    if (!editable) event.preventDefault();
  });

  await onToast(toast);
  await onAutotypeChoose(openChooser);

  nav.unlock(await api.unlockView());
  if (info.notice) toast(info.notice);
}

start().catch((err) => {
  const root = document.getElementById("app");
  if (root) setChildren(root, el("div", { class: "fatal" }, "Our Logins could not start: ", errorText(err)));
});

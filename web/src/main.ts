import { api, errorText, onSaved, onToast, type PlatformInfo } from "./api";
import { isOverlayOpen } from "./dialogs";
import { el, setChildren } from "./dom";
import { registerFluent } from "./fluent";
import { nav } from "./nav";
import { initTheme } from "./theme";
import { toast } from "./toast";
import { mainKeydown, renderMain } from "./views/main";
import { startQuick } from "./views/quick";
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
  initTheme(info.os);

  // The webview's own context menu (Back, Reload, Print, Inspect) does not belong in a desktop
  // app. Text fields keep it for cut/copy/paste.
  document.addEventListener("contextmenu", (event) => {
    const editable = event.composedPath().some(
      (node) => node instanceof HTMLElement && (node.matches("input, textarea") || node.isContentEditable),
    );
    if (!editable) event.preventDefault();
  });

  // The same bundle runs the Quick Access popup (tauri.conf.json loads it with ?quick).
  if (new URLSearchParams(location.search).has("quick")) {
    document.documentElement.dataset.backdrop = "false";
    await startQuick(root);
    return;
  }
  document.documentElement.dataset.backdrop = String(info.backdrop);

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

  await onToast(toast);
  // Saving indicators and save errors live in the main view.
  await onSaved(() => {
    if (page === "main") api.mainView().then(nav.main, (err) => toast(errorText(err)));
  });

  nav.unlock(await api.unlockView());
  if (info.notice) toast(info.notice);
}

start().catch((err) => {
  const root = document.getElementById("app");
  if (root) setChildren(root, el("div", { class: "fatal" }, "Our Logins could not start: ", errorText(err)));
});

import { api, errorText, onAutoLock, onAutoLocked, onCloseRequested, onSaved, onToast, type PlatformInfo } from "./api";
import { isOverlayOpen } from "./dialogs";
import { el, setChildren } from "./dom";
import { registerFluent } from "./fluent";
import { nav } from "./nav";
import { initTheme } from "./theme";
import { toast } from "./toast";
import { autoLock, autoLocked, closeRequested, mainKeydown, noteInput, renderMain } from "./views/main";
import { startQuick } from "./views/quick";
import { checkForUpdates } from "./update";
import { leaveUnlock, renderUnlock, unlockKeydown } from "./views/unlock";

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

  // Auto-lock counts from the last use of the app: input in either window is reported, at
  // most every 30 s. A lost report only means auto-lock may come a little early. Every
  // input also restarts the clock for saving the details pane's edits while the user is
  // away (main window only; Quick Access never has edits).
  let reported = 0;
  const noteActivity = (event: Event) => {
    noteInput(event.type === "keydown" || event.type === "pointerdown");
    const now = Date.now();
    if (now - reported < 30_000) return;
    reported = now;
    api.noteActivity().catch(() => {});
  };
  for (const type of ["keydown", "pointerdown", "pointermove", "wheel"]) {
    document.addEventListener(type, noteActivity, { capture: true, passive: true });
  }

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
    if (page === "unlock") leaveUnlock();
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
  // Closing the window with unsaved edits in the details pane: Save / Discard / Stay first.
  await onCloseRequested(() => void closeRequested());
  // Auto-lock: the page saves edits in the details pane, then everything locks.
  await onAutoLock(() => void autoLock());
  await onAutoLocked(() => void autoLocked());

  // WebView2 starts without keyboard focus; take it so typing reaches the password box.
  await api.focusWindow();
  const view = await api.unlockView();
  // Nothing to unlock yet: go straight to the file picker.
  nav.unlock(view.recent.length === 0 && view.openDbs.length === 0 ? await api.browseOpen() : view);
  if (info.notice) toast(info.notice);
  // Later than the startup notice, so the update offer does not replace it.
  window.setTimeout(() => void checkForUpdates(false), 5000);
}

start().catch((err) => {
  const root = document.getElementById("app");
  if (root) setChildren(root, el("div", { class: "fatal" }, "Our Logins could not start: ", errorText(err)));
});

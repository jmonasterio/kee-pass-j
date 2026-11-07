import { api, errorText, type SettingsView } from "../api";
import { openDialog } from "../dialogs";
import { button, el, icon, type IconName } from "../dom";
import { nav } from "../nav";
import { toast } from "../toast";

function toggle(label: string, checked: boolean): HTMLElementTagNameMap["fluent-switch"] {
  return el("fluent-switch", { attrs: { checked, "aria-label": label } });
}

/** One Windows 11 Settings-style row: icon, title with a short explanation, control on the right. */
function row(iconName: IconName | null, title: string, desc: string, ...controls: HTMLElement[]): HTMLElement {
  return el(
    "div",
    { class: "setting-row" },
    el("div", { class: "setting-icon" }, iconName ? icon(iconName) : null),
    el("div", { class: "setting-text" }, el("div", { class: "setting-title" }, title), el("div", { class: "setting-desc" }, desc)),
    el("div", { class: "setting-control" }, controls),
  );
}

function section(title: string, ...rows: (HTMLElement | null)[]): HTMLElement {
  return el("section", { class: "settings-section" }, el("h3", { class: "settings-section-title" }, title), el("div", { class: "settings-card" }, rows));
}

/** Opens the Settings dialog. */
export async function openSettings(): Promise<void> {
  if (document.querySelector("fluent-dialog.settings-dialog")) return;
  let current: SettingsView;
  try {
    current = await api.settingsGet();
  } catch (err) {
    toast(errorText(err));
    return;
  }

  const hotkey = el("fluent-text-input", {
    class: "hotkey-input",
    value: current.autotypeHotkey,
    placeholder: "Ctrl+Alt+A",
    attrs: { spellcheck: "false", autofocus: true, "aria-label": "Global hotkey" },
  });
  const sequence = el("fluent-text-input", {
    class: "mono sequence-input",
    value: current.autotypeDefaultSequence,
    placeholder: "{USERNAME}{TAB}{PASSWORD}{ENTER}",
    attrs: { spellcheck: "false", "aria-label": "Default typing sequence" },
  });
  const reopenLast = toggle("Reopen the last database", current.reopenLast);
  const folderColors = toggle("Colour tiles by folder", current.tileColors === "folder");
  const notesMonospace = toggle("Monospace notes", current.notesMonospace);
  const quickUnlock = current.quickUnlockName ? toggle(`Unlock with ${current.quickUnlockName}`, current.quickUnlock) : null;
  const clearChoices = [0, 10, 20, 30, 60, 120];
  if (!clearChoices.includes(current.clipboardClearSeconds)) clearChoices.push(current.clipboardClearSeconds);
  const clipboardClear = el(
    "fluent-dropdown",
    { class: "clipboard-dropdown", attrs: { "aria-label": "Clear the clipboard after" } },
    el(
      "fluent-listbox",
      null,
      clearChoices
        .sort((a, b) => a - b)
        .map((s) =>
          el("fluent-option", { attrs: { value: String(s), selected: s === current.clipboardClearSeconds } }, s === 0 ? "Never" : `${s} seconds`),
        ),
    ),
  );

  const length = Math.min(128, Math.max(4, current.generator.length));
  const lengthLabel = el("span", { class: "slider-value" }, String(length));
  const slider = el("fluent-slider", {
    class: "length-slider",
    attrs: { min: "4", max: "128", step: "1", value: String(length), "aria-label": "Default password length" },
    on: { change: () => (lengthLabel.textContent = slider.value) },
  });
  const digits = toggle("Digits", current.generator.digits);
  const symbols = toggle("Symbols", current.generator.symbols);
  const uppercase = toggle("Uppercase letters", current.generator.uppercase);
  const excludeAmbiguous = toggle("Avoid look-alike characters", current.generator.excludeAmbiguous);
  const error = el("div", { class: "form-error", attrs: { role: "alert" } });

  const saveButton = button("Save", () => void save(), { appearance: "primary" });
  let busy = false;
  const save = async () => {
    if (busy) return;
    const settings: SettingsView = {
      autotypeHotkey: hotkey.value.trim(),
      autotypeDefaultSequence: sequence.value.trim(),
      reopenLast: reopenLast.checked,
      tileColors: folderColors.checked ? "folder" : "off",
      notesMonospace: notesMonospace.checked,
      generator: {
        length: Number(slider.value) || length,
        digits: digits.checked,
        symbols: symbols.checked,
        uppercase: uppercase.checked,
        excludeAmbiguous: excludeAmbiguous.checked,
      },
      backupDir: current.backupDir,
      quickUnlock: quickUnlock ? quickUnlock.checked : current.quickUnlock,
      clipboardClearSeconds: Number(clipboardClear.value),
      quickUnlockName: current.quickUnlockName,
    };
    busy = true;
    saveButton.disabled = true;
    error.textContent = "";
    try {
      await api.settingsSave(settings);
      handle.close();
      if (document.querySelector(".main-page")) nav.main(await api.mainView());
    } catch (err) {
      error.textContent = errorText(err);
    } finally {
      busy = false;
      saveButton.disabled = false;
    }
  };

  const body = el(
    "div",
    { class: "settings-body" },
    section(
      "Auto-Type",
      row(
        "keyboard",
        "Global hotkey",
        "In a login window: types the matching account, or opens Quick Access to pick or search one.",
        hotkey,
      ),
      row("text", "Default typing sequence", "What Auto-Type types when an account has no sequence of its own.", sequence),
    ),
    section(
      "General",
      row("refresh", "Reopen the last database", "Have it selected on the unlock screen when the app starts.", reopenLast),
      quickUnlock
        ? row(
            "key",
            `Unlock with ${current.quickUnlockName}`,
            `After you unlock a database with its password once, ${current.quickUnlockName} unlocks it next time. Turning this off forgets the kept passwords.`,
            quickUnlock,
          )
        : null,
      row("text", "Monospace notes", "Show notes in a fixed-width font, so columns line up.", notesMonospace),
      row(
        "copy",
        "Clear the clipboard",
        "After you copy a password, empty the clipboard this long afterwards, unless you copied something else meanwhile.",
        clipboardClear,
      ),
      row(
        "folder",
        "Colour tiles by folder",
        "Each account's letter tile takes its folder's colour. Pick a folder's colour from its ⋯ menu in the sidebar.",
        folderColors,
      ),
    ),
    section(
      "New passwords",
      row("key", "Length", "How many characters a generated password has.", slider, lengthLabel),
      row(null, "Digits", "Include 0–9.", digits),
      row(null, "Symbols", "Include ! @ # $ % & * ? - _ = +", symbols),
      row(null, "Uppercase letters", "Include A–Z as well as a–z.", uppercase),
      row(null, "Avoid look-alike characters", "Leave out 0 O o 1 l I | 5 S 8 B 2 Z, so passwords are easy to read and type.", excludeAmbiguous),
    ),
    section(
      "Backups",
      row(
        "folder",
        "Backup folder",
        current.backupDir,
        button("Open folder", async () => {
          try {
            await api.openBackupFolder();
          } catch (err) {
            error.textContent = errorText(err);
          }
        }),
      ),
    ),
    error,
  );
  body.addEventListener("keydown", (event: KeyboardEvent) => {
    if (event.key === "Enter" && !event.isComposing && (event.target as HTMLElement | null)?.tagName === "FLUENT-TEXT-INPUT") {
      event.preventDefault();
      void save();
    }
  });

  const handle = openDialog({
    title: "Settings",
    class: "settings-dialog",
    body,
    actions: [button("Cancel", () => handle.close()), saveButton],
  });
}

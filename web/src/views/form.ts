import { api, errorText, type CustomField, type EntryForm, type FormData, type FormFiles, type FormMode, type Id } from "../api";
import { onClosed } from "../dialogs";
import { button, debounce, el, field, setChildren } from "../dom";
import { BACKUP_COPIES, FILE_ADVICE, formatBytes, LARGE_FILE, LARGE_TOTAL } from "../files";
import { passwordBox, setSecret } from "../password-box";
import { nav } from "../nav";
import { copyAndTell, toast } from "../toast";

type TextInputEl = HTMLElementTagNameMap["fluent-text-input"];
type TextAreaEl = HTMLElementTagNameMap["fluent-textarea"];
type CheckboxEl = HTMLElementTagNameMap["fluent-checkbox"];

function textInput(value: string, opts: { placeholder?: string; class?: string; autofocus?: boolean } = {}): TextInputEl {
  return el("fluent-text-input", {
    class: opts.class,
    value,
    placeholder: opts.placeholder ?? "",
    attrs: { autofocus: opts.autofocus },
  });
}

function textArea(value: string, opts: { placeholder?: string; class?: string; secret?: boolean } = {}): TextAreaEl {
  return el("fluent-textarea", {
    class: opts.class,
    value,
    placeholder: opts.placeholder ?? "",
    resize: "vertical",
    attrs: { spellcheck: opts.secret ? "false" : undefined, autocomplete: opts.secret ? "off" : undefined },
  });
}

function checkbox(label: string, checked: boolean, onChange: (checked: boolean) => void): { field: HTMLElement; box: CheckboxEl } {
  const box = el("fluent-checkbox", { attrs: { checked }, on: { change: () => onChange(box.checked) } });
  return { field: field(label, box, { position: "after" }), box };
}

interface CustomRow {
  row: HTMLElement;
  key: TextInputEl;
  value: TextInputEl;
  prot: CheckboxEl;
}

/** Opens the Add/Edit account drawer. */
export async function openForm(mode: FormMode, notesMonospace: boolean): Promise<void> {
  if (document.querySelector("fluent-drawer")) return;
  let data: FormData;
  try {
    data = await api.formOpen(mode);
  } catch (err) {
    toast(errorText(err));
    return;
  }
  const entry = data.entry;
  const editing = data.mode === "edit";
  // Document entries: a name, free text (plain or Markdown) and files; no login fields.
  const isDoc = entry.document;
  const noun = isDoc ? "document" : "account";

  const title = textInput(entry.title, { autofocus: true, placeholder: isDoc ? "e.g. Passport" : "e.g. Netflix" });
  const url = textInput(entry.url, { placeholder: "netflix.com" });
  const username = textInput(entry.username);
  // The password is generated, so it starts in clear text like a normal field.
  const password = passwordBox({ value: entry.password, shown: true, large: true });
  const recoveryPin = passwordBox({ value: entry.recoveryPin });
  const recoveryPhone = textInput(entry.recoveryPhone);
  const securityQuestions = textArea(entry.securityQuestions, { secret: true });
  const notes = textArea(entry.notes, {
    class: [notesMonospace ? "mono" : "", isDoc ? "doc-text" : ""].join(" ").trim() || undefined,
    placeholder: isDoc ? "Plain text or Markdown: # headings, **bold**, - lists, [links](https://…)" : "",
  });
  const error = el("div", { class: "form-error", attrs: { role: "alert" } });

  // Generator.
  const gen = { ...data.gen, length: Math.min(64, Math.max(8, data.gen.length)) };
  let genSeq = 0;
  const regenerate = async () => {
    const seq = ++genSeq;
    try {
      const next = await api.generate({ ...gen });
      if (seq === genSeq) password.value = next;
    } catch (err) {
      if (seq === genSeq) toast(errorText(err));
    }
  };
  const lengthLabel = el("span", { class: "slider-value" }, String(gen.length));
  const slider = el("fluent-slider", {
    class: "length-slider",
    attrs: { min: "8", max: "64", step: "1", value: String(gen.length), "aria-label": "Password length" },
    on: {
      change: () => {
        const length = Number(slider.value);
        if (!Number.isFinite(length) || length === gen.length) return;
        gen.length = length;
        lengthLabel.textContent = String(length);
        void regenerate();
      },
    },
  });
  const digits = checkbox("Digits", gen.digits, (on) => {
    gen.digits = on;
    void regenerate();
  });
  const symbols = checkbox("Symbols", gen.symbols, (on) => {
    gen.symbols = on;
    void regenerate();
  });

  // Duplicate detection.
  const dupBar = el("div", { class: "dup-slot" });
  let dupSeq = 0;
  const checkDuplicate = debounce(async () => {
    const seq = ++dupSeq;
    try {
      const dup = await api.findDuplicate(title.value.trim(), url.value.trim(), entry.id);
      if (seq !== dupSeq) return;
      if (!dup) {
        dupBar.replaceChildren();
        return;
      }
      const replace = button("Replace", () => void save(dup.id), { size: "small" });
      replace.setAttribute("slot", "actions");
      dupBar.replaceChildren(
        el(
          "fluent-message-bar",
          { intent: "warning", layout: "multiline" },
          `Already have "${dup.title}". Replace it? The old entry will be kept and marked not working.`,
          replace,
        ),
      );
    } catch {
      // Duplicate hints are advisory; a failed lookup just shows no hint.
      if (seq === dupSeq) dupBar.replaceChildren();
    }
  }, 200);
  title.addEventListener("input", checkDuplicate);
  url.addEventListener("input", checkDuplicate);

  // Group.
  const group = el(
    "fluent-dropdown",
    { class: "group-dropdown" },
    el(
      "fluent-listbox",
      null,
      data.groups.map((g) =>
        el(
          "fluent-option",
          {
            class: g.isRecycleBin ? "bin" : undefined,
            style: { "padding-inline-start": `${8 + g.depth * 16}px` },
            attrs: { value: g.id, selected: g.id === entry.group },
          },
          g.name,
        ),
      ),
    ),
  );

  // Edit-only fields.
  const tags = textInput(entry.tags, { placeholder: "Comma separated" });
  const customRows: CustomRow[] = [];
  const customList = el("div", { class: "custom-list" });
  const addCustomRow = (cf: CustomField) => {
    const key = textInput(cf.key, { placeholder: "Name" });
    const value = passwordBox({ value: cf.value, placeholder: "Value" });
    setSecret(value, cf.protected);
    const prot = el("fluent-checkbox", { attrs: { checked: cf.protected } });
    prot.addEventListener("change", () => setSecret(value, prot.checked));
    const item: CustomRow = {
      key,
      value,
      prot,
      row: el(
        "div",
        { class: "custom-row" },
        key,
        value,
        field("Protected", prot, { position: "after" }),
        button(
          null,
          () => {
            customRows.splice(customRows.indexOf(item), 1);
            item.row.remove();
          },
          { icon: "dismiss", appearance: "subtle", title: "Remove field" },
        ),
      ),
    };
    customRows.push(item);
    customList.appendChild(item.row);
    return item;
  };
  for (const cf of entry.custom) addCustomRow(cf);
  const sequence = textInput(entry.autotypeSequence, { placeholder: "{USERNAME}{TAB}{PASSWORD}{ENTER}", class: "mono" });
  sequence.setAttribute("spellcheck", "false");
  const windows = textArea(entry.autotypeWindows, { class: "mono", placeholder: "One window title pattern per line" });
  windows.setAttribute("spellcheck", "false");

  const saveButton = button(editing ? "Save changes" : `Save ${noun}`, () => void save(null), { appearance: "primary" });
  let busy = false;
  let closed = false;
  const save = async (replace: Id | null) => {
    if (busy || closed) return;
    const name = title.value.trim();
    if (!name) {
      error.textContent = `Give the ${noun} a name first.`;
      title.focus();
      return;
    }
    const form: EntryForm = {
      id: entry.id,
      group: group.value || entry.group,
      title: name,
      url: url.value.trim(),
      username: username.value,
      password: password.value,
      recoveryPin: recoveryPin.value,
      recoveryPhone: recoveryPhone.value,
      securityQuestions: securityQuestions.value,
      notes: notes.value,
      tags: editing ? tags.value : entry.tags,
      autotypeSequence: editing ? sequence.value.trim() : entry.autotypeSequence,
      autotypeWindows: editing ? windows.value : entry.autotypeWindows,
      custom: editing
        ? customRows
            .map((r) => ({ key: r.key.value.trim(), value: r.value.value, protected: r.prot.checked }))
            .filter((c) => c.key !== "" || c.value !== "")
        : entry.custom,
      document: entry.document,
    };
    busy = true;
    saveButton.disabled = true;
    error.textContent = "";
    try {
      const view = await api.formSave(form, replace);
      closed = true;
      drawer.hide();
      nav.main(view);
    } catch (err) {
      error.textContent = errorText(err);
    } finally {
      busy = false;
      saveButton.disabled = false;
    }
  };

  const closeButton = button(null, () => {}, { icon: "dismiss", appearance: "subtle", title: "Close" });
  closeButton.setAttribute("slot", "close");
  const cancelButton = button("Cancel", () => drawer.hide());
  const footer = el("div", { class: "drawer-footer", attrs: { slot: "footer" } }, cancelButton, saveButton);

  // Built lazily: the shared inputs (title, notes, group, tags) can sit in only one field.
  const accountFields = () => [
    field("Site or service", title),
    field("Website", url),
    dupBar,
    field("Username or email", username),
    el(
      "div",
      { class: "password-block" },
      field("Password", password),
      el(
        "div",
        { class: "password-actions" },
        button("Regenerate", () => void regenerate(), { icon: "refresh" }),
        button("Copy", () => void copyAndTell(api.copyText(password.value)), { icon: "copy" }),
      ),
      el(
        "div",
        { class: "generator" },
        el("div", { class: "slider-line" }, el("span", { class: "generator-label" }, "Length"), slider, lengthLabel),
        el("div", { class: "checkbox-line" }, digits.field, symbols.field),
      ),
    ),
    el("div", { class: "two-col" }, field("Recovery PIN", recoveryPin), field("Recovery phone", recoveryPhone)),
    field("Security questions", securityQuestions),
    field("Notes", notes),
    field("Folder", group),
    editing
      ? [
          field("Tags", tags),
          el(
            "div",
            { class: "form-section" },
            el("div", { class: "section-label" }, "Custom fields"),
            customList,
            button("Add field", () => addCustomRow({ key: "", value: "", protected: false }).key.focus(), {
              icon: "add",
              appearance: "subtle",
              class: "align-start",
            }),
          ),
          el(
            "div",
            { class: "form-section" },
            el("div", { class: "section-label" }, "Auto-Type"),
            field("Sequence (empty = default from Settings)", sequence),
            field("Window titles", windows),
          ),
        ]
      : null,
  ];
  const documentFields = () => [
    field("Name", title),
    field("Text", notes),
    editing ? null : filesSection(),
    field("Folder", group),
    editing ? field("Tags", tags) : null,
  ];
  const content = el("div", { class: "form-content" }, isDoc ? documentFields() : accountFields(), error);

  const heading = `${editing ? "Edit" : "Add"} ${noun}` + (data.database ? ` ${editing ? "in" : "to"} ${data.database}` : "");
  const drawer = el(
    "fluent-drawer",
    { class: "form-drawer", attrs: { position: "end", "aria-label": heading } },
    el(
      "fluent-drawer-body",
      null,
      el("h2", { class: "dialog-title", attrs: { slot: "title" } }, heading),
      closeButton,
      content,
      footer,
    ),
  );
  drawer.addEventListener("keydown", (event: KeyboardEvent) => {
    if (event.key !== "Enter" || event.isComposing) return;
    const target = event.target as HTMLElement | null;
    if (event.ctrlKey || event.metaKey) {
      event.preventDefault();
      void save(null);
    } else if (target?.tagName === "FLUENT-TEXT-INPUT") {
      event.preventDefault();
      void save(null);
    }
  });
  onClosed(drawer, () => {
    closed = true;
  });
  document.body.appendChild(drawer);
  drawer.show();
}

/** "Files" in the Add document form: files picked here are attached when the form is saved. */
function filesSection(): HTMLElement {
  const list = el("div", { class: "form-file-list" });
  const note = el("div", { class: "muted small" }, FILE_ADVICE);
  const show = (f: FormFiles) => {
    setChildren(
      list,
      f.files.map((file, index) =>
        el(
          "div",
          { class: "attachment-row" },
          el("span", { class: "attachment-name", attrs: { title: file.name } }, file.name),
          el("span", { class: "attachment-size" }, formatBytes(file.size)),
          button(null, () => void update(api.formRemoveFile(index)), { icon: "dismiss", appearance: "subtle", size: "small", title: `Remove ${file.name}` }),
        ),
      ),
    );
    const adding = f.files.reduce((sum, file) => sum + file.size, 0);
    const after = f.attachmentsBytes + adding;
    const large = f.files.some((file) => file.size > LARGE_FILE) || after > LARGE_TOTAL;
    note.classList.toggle("storage-warn", large);
    if (f.files.length === 0) {
      note.textContent = FILE_ADVICE;
    } else if (large) {
      note.textContent =
        `Adding ${formatBytes(adding)}; files in this database will total ${formatBytes(after)}. ` +
        `Every change saves the whole database again, and the backup folder keeps up to ${BACKUP_COPIES} copies of it ` +
        `(about ${formatBytes((f.fileBytes + adding) * BACKUP_COPIES)}). A photo of a paper is usually much smaller scanned as a PDF; ` +
        "many large files fit better in a separate database.";
    } else {
      note.textContent = `Adding ${formatBytes(adding)}; files in this database will total ${formatBytes(after)}.`;
    }
  };
  const update = async (call: Promise<FormFiles>) => {
    try {
      show(await call);
    } catch (err) {
      toast(errorText(err));
    }
  };
  return el(
    "div",
    { class: "form-files" },
    el(
      "div",
      { class: "field-label-line" },
      el("label", { class: "form-files-label" }, "Files"),
      button("Add files…", () => void update(api.formPickFiles()), { icon: "add", appearance: "subtle", size: "small" }),
    ),
    list,
    note,
  );
}

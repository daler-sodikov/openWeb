// Run popups: the trust question (with the exact command), the
// configuration chooser (Alt+Shift+F10) and the configuration editor.
import type { PackageManager, RunConfig, RunConfigurations } from "../ipc/bindings";
import { bindListKeys, input, Popup } from "../popup/popup";
import { VirtualList } from "../ui/virtual-list";
import { type ConfigForm, type FormKind, fromForm, KIND_LABELS, toForm } from "./config-form";

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  testid?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (testid) node.dataset.testid = testid;
  return node;
}

function actionButton(label: string, testid: string): HTMLButtonElement {
  const b = el("button", "popup-button", testid);
  b.type = "button";
  b.textContent = label;
  return b;
}

/** Asks whether the project may run commands; nothing runs before. */
export class TrustDialog extends Popup {
  private answered = false;

  constructor(
    command: string,
    cwd: string,
    private readonly answer: (trusted: boolean) => void,
  ) {
    super("trust-dialog", "trust-dialog", "Trust this project?");
    const title = el("div", "popup-title");
    title.textContent = "Trust this project?";
    const text = el("p", "trust-text");
    text.textContent =
      "Nothing in this project has run yet. Running it executes this command on your computer:";
    const pre = el("pre", "trust-command", "trust-command");
    pre.textContent = command;
    const where = el("p", "trust-cwd", "trust-cwd");
    where.textContent = `in ${cwd}`;
    const note = el("p", "trust-text");
    note.textContent =
      "Trust it only if you trust the project's code and configuration. Ferrule remembers the answer for this project.";
    const buttons = el("div", "popup-buttons");
    const trust = actionButton("Trust this project and run", "trust-confirm");
    const cancel = actionButton("Cancel", "trust-cancel");
    buttons.append(trust, cancel);
    this.element.append(title, text, pre, where, note, buttons);
    trust.addEventListener("click", () => {
      this.finish(true);
    });
    cancel.addEventListener("click", () => {
      this.finish(false);
    });
  }

  override open(): void {
    super.open();
    this.element.querySelector<HTMLButtonElement>('[data-testid="trust-cancel"]')?.focus();
  }

  /** Escape or a click outside counts as Cancel. */
  override close(): void {
    super.close();
    this.finish(false);
  }

  private finish(trusted: boolean): void {
    if (this.answered) return;
    this.answered = true;
    this.close();
    this.answer(trusted);
  }
}

interface ChooserItem {
  config: RunConfig;
  group: "saved" | "detected";
}

export interface ChooserActions {
  run(config: RunConfig): void;
  edit(config: RunConfig | null): void;
}

function kindLabel(config: RunConfig): string {
  const kind = config.kind;
  return KIND_LABELS[kind.type === "test" ? kind.framework : kind.type];
}

/** Stored and detected configurations; Enter runs, F4 edits. */
export class ConfigChooser extends Popup {
  private readonly field = input("popup-input", "run-config-filter", "Run configuration");
  private readonly list: VirtualList<ChooserItem>;
  private readonly footer = el("div", "popup-footer", "run-config-footer");
  private readonly all: ChooserItem[];
  private readonly note: string;

  constructor(
    configs: RunConfigurations,
    private readonly actions: ChooserActions,
  ) {
    super("run-chooser", "run-chooser", "Run Configurations");
    this.all = [
      ...configs.stored.map((config) => ({ config, group: "saved" as const })),
      ...configs.detected.map((config) => ({ config, group: "detected" as const })),
    ];
    this.note = configs.error ? `.ferrule/run.json: ${configs.error}` : "Enter: run · F4: edit";
    const title = el("div", "popup-title");
    title.textContent = "Run";
    this.list = new VirtualList<ChooserItem>(
      26,
      (item) => {
        const row = el("div", "run-config-row", "run-config-item");
        row.dataset.id = item.config.id;
        const name = el("span", "se-name");
        name.textContent = item.config.name;
        const detail = el("span", "se-detail");
        detail.textContent = `${kindLabel(item.config)} · ${item.group}`;
        row.append(name, detail);
        return row;
      },
      (item) => {
        this.close();
        this.actions.run(item.config);
      },
      "run-config-list",
    );
    const buttons = el("div", "popup-buttons");
    const edit = actionButton("Edit…", "run-config-edit");
    const add = actionButton("New…", "run-config-new");
    edit.addEventListener("click", () => {
      this.editSelected();
    });
    add.addEventListener("click", () => {
      this.close();
      this.actions.edit(null);
    });
    buttons.append(edit, add);
    this.element.append(title, this.field, this.list.element, buttons, this.footer);
    bindListKeys(this.field, this.list);
    this.field.addEventListener("keydown", (event) => {
      if (event.key === "F4") {
        event.preventDefault();
        this.editSelected();
      }
    });
    this.field.addEventListener("input", () => {
      this.filter();
    });
  }

  override open(): void {
    super.open();
    this.filter();
    this.field.focus();
  }

  private filter(): void {
    const query = this.field.value.trim().toLowerCase();
    const items = this.all.filter((i) => i.config.name.toLowerCase().includes(query));
    this.list.setItems(items);
    if (items.length > 0) this.footer.textContent = this.note;
    else if (query) this.footer.textContent = "No configuration matches.";
    else
      this.footer.textContent =
        "No configurations. Add a package.json script or create one with New….";
  }

  private editSelected(): void {
    const item = this.list.selectedItem();
    this.close();
    this.actions.edit(item?.config ?? null);
  }
}

export interface EditorActions {
  save(config: RunConfig): Promise<RunConfig | null>;
  remove(id: string): Promise<boolean>;
  run(config: RunConfig): void;
}

const MANAGERS: readonly PackageManager[] = ["bun", "pnpm", "yarn", "npm"];

/** Edits one configuration; saving writes `.ferrule/run.json` (in Rust). */
export class ConfigEditor extends Popup {
  private readonly error = el("div", "popup-footer run-config-error", "run-config-error");

  constructor(
    private readonly base: RunConfig,
    detectedManager: PackageManager,
    private readonly actions: EditorActions,
  ) {
    super("run-editor", "run-editor", "Edit Run Configuration");
    const stored = base.id !== "" && !base.id.startsWith("detected:");
    const form = toForm(base);
    const title = el("div", "popup-title");
    title.textContent = stored ? "Edit Run Configuration" : "New Run Configuration";
    const grid = el("div", "run-form");
    const name = this.text(grid, "Name", "run-form-name", form.name);
    const kind = el("select", "run-form-input", "run-form-kind");
    for (const [value, label] of Object.entries(KIND_LABELS)) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = label;
      kind.append(option);
    }
    kind.value = form.kind;
    this.row(grid, "Type", kind);
    const target = this.text(grid, "Script", "run-form-target", form.target);
    const files = this.area(
      grid,
      "Test files (one per line; empty: all)",
      "run-form-files",
      form.files,
    );
    const cwd = this.text(grid, "Working directory", "run-form-cwd", form.cwd);
    const args = this.area(grid, "Arguments (one per line)", "run-form-args", form.args);
    const env = this.area(grid, "Environment (NAME=value per line)", "run-form-env", form.env);
    const manager = el("select", "run-form-input", "run-form-manager");
    const detected = document.createElement("option");
    detected.value = "";
    detected.textContent = `Detected (${detectedManager})`;
    manager.append(detected);
    for (const m of MANAGERS) {
      const option = document.createElement("option");
      option.value = m;
      option.textContent = m;
      manager.append(option);
    }
    manager.value = form.packageManager;
    const managerRow = this.row(grid, "Package manager", manager);

    const sync = () => {
      const k = kind.value as FormKind;
      const targetRow = target.parentElement;
      if (targetRow) {
        targetRow.hidden = k !== "node" && k !== "npmScript";
        const label = targetRow.querySelector("label");
        if (label) label.textContent = k === "node" ? "File" : "Script";
      }
      const filesRow = files.parentElement;
      if (filesRow) filesRow.hidden = k === "node" || k === "npmScript";
      managerRow.hidden = k !== "npmScript";
    };
    kind.addEventListener("change", sync);
    sync();

    const read = (): ConfigForm => ({
      name: name.value,
      kind: kind.value as FormKind,
      target: target.value,
      files: files.value,
      cwd: cwd.value,
      args: args.value,
      env: env.value,
      packageManager: manager.value as PackageManager | "",
    });
    const build = (): RunConfig | null => {
      const result = fromForm(read(), this.base);
      if ("error" in result) {
        this.error.textContent = result.error;
        return null;
      }
      this.error.textContent = "";
      return result.config;
    };

    const buttons = el("div", "popup-buttons");
    const run = actionButton("Run", "run-form-run");
    const save = actionButton("Save", "run-form-save");
    const remove = actionButton("Delete", "run-form-delete");
    const cancel = actionButton("Cancel", "run-form-cancel");
    remove.hidden = !stored;
    buttons.append(run, save, remove, cancel);
    save.addEventListener("click", () => {
      const config = build();
      if (!config) return;
      void this.actions.save(config).then((saved) => {
        if (saved) this.close();
      });
    });
    run.addEventListener("click", () => {
      const config = build();
      if (!config) return;
      this.close();
      // Unsaved: run what the form shows, not a stored version.
      this.actions.run(stored ? config : { ...config, id: "adhoc:edited" });
    });
    remove.addEventListener("click", () => {
      void this.actions.remove(this.base.id).then((removed) => {
        if (removed) this.close();
      });
    });
    cancel.addEventListener("click", () => {
      this.close();
    });
    this.element.append(title, grid, this.error, buttons);
  }

  override open(): void {
    super.open();
    this.element.querySelector<HTMLInputElement>('[data-testid="run-form-name"]')?.focus();
  }

  /** Shows an error from saving or deleting. */
  showError(message: string): void {
    this.error.textContent = message;
  }

  private row(grid: HTMLElement, label: string, control: HTMLElement): HTMLElement {
    const row = el("div", "run-form-row");
    const text = el("label", "run-form-label");
    text.textContent = label;
    row.append(text, control);
    grid.append(row);
    return row;
  }

  private text(grid: HTMLElement, label: string, testid: string, value: string): HTMLInputElement {
    const field = input("run-form-input", testid, "");
    field.value = value;
    this.row(grid, label, field);
    return field;
  }

  private area(
    grid: HTMLElement,
    label: string,
    testid: string,
    value: string,
  ): HTMLTextAreaElement {
    const field = el("textarea", "run-form-input", testid);
    field.rows = 2;
    field.spellcheck = false;
    field.value = value;
    this.row(grid, label, field);
    return field;
  }
}

import type { RefactoringPreview } from "../ipc/bindings";
import { errorMessage } from "../ipc/result";
import { input, Popup } from "./popup";

export class RenamePopup extends Popup {
  private readonly field = input("popup-input", "rename-input", "New name");
  private readonly status = document.createElement("div");
  private readonly files = document.createElement("div");
  private readonly previewButton = document.createElement("button");
  private readonly applyButton = document.createElement("button");
  private preview: RefactoringPreview | null = null;
  private sequence = 0;

  constructor(
    initial: string,
    private readonly previewRename: (name: string) => Promise<RefactoringPreview>,
    private readonly applyRename: (id: number) => Promise<void>,
  ) {
    super("rename", "rename-dialog", "Rename Symbol");
    const title = document.createElement("div");
    title.className = "popup-title";
    title.textContent = "Rename Symbol";
    this.field.value = initial;
    this.previewButton.type = "button";
    this.previewButton.className = "toolbar-button";
    this.previewButton.textContent = "Preview Rename";
    this.previewButton.dataset.testid = "rename-preview";
    this.previewButton.addEventListener("click", () => {
      void this.buildPreview();
    });
    this.applyButton.type = "button";
    this.applyButton.className = "toolbar-button";
    this.applyButton.textContent = "Apply Rename";
    this.applyButton.dataset.testid = "rename-apply";
    this.applyButton.disabled = true;
    this.applyButton.addEventListener("click", () => {
      void this.apply();
    });
    const buttons = document.createElement("div");
    buttons.className = "rename-buttons";
    buttons.append(this.previewButton, this.applyButton);
    this.status.className = "popup-footer";
    this.status.dataset.testid = "rename-status";
    this.status.setAttribute("role", "status");
    this.files.className = "rename-files";
    this.files.dataset.testid = "rename-files";
    this.element.append(title, this.field, buttons, this.status, this.files);
    this.field.addEventListener("input", () => {
      this.invalidatePreview();
    });
    this.field.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        void this.buildPreview();
      }
    });
  }

  override open(): void {
    super.open();
    this.field.focus();
    this.field.select();
  }

  override close(): void {
    this.sequence++;
    super.close();
  }

  private invalidatePreview(): void {
    this.sequence++;
    this.preview = null;
    this.applyButton.disabled = true;
    this.files.replaceChildren();
    this.status.textContent = "Preview the changes before applying them.";
  }

  private async buildPreview(): Promise<void> {
    const name = this.field.value.trim();
    if (!name) {
      this.status.textContent = "Enter a new name.";
      return;
    }
    const sequence = ++this.sequence;
    this.preview = null;
    this.applyButton.disabled = true;
    this.status.textContent = "Preparing preview…";
    this.files.replaceChildren();
    try {
      const preview = await this.previewRename(name);
      if (sequence !== this.sequence || this.isClosed) return;
      this.preview = preview;
      this.applyButton.disabled = preview.total === 0;
      this.status.textContent = `${String(preview.total)} edits in ${String(preview.files.length)} files`;
      this.files.replaceChildren(
        ...preview.files.map((file) => {
          const row = document.createElement("div");
          row.className = "rename-file";
          const path = document.createElement("span");
          path.textContent = file.path;
          const count = document.createElement("span");
          count.textContent = `${String(file.count)} edits`;
          row.append(path, count);
          if (file.readOnly) {
            row.classList.add("read-only");
            row.title = "Library files are not changed.";
          }
          return row;
        }),
      );
    } catch (error: unknown) {
      if (sequence === this.sequence) this.status.textContent = errorMessage(error);
    }
  }

  private async apply(): Promise<void> {
    const preview = this.preview;
    if (!preview) return;
    this.applyButton.disabled = true;
    this.status.textContent = "Applying rename…";
    try {
      await this.applyRename(preview.id);
      this.close();
    } catch (error: unknown) {
      this.status.textContent = errorMessage(error);
      this.applyButton.disabled = false;
    }
  }
}

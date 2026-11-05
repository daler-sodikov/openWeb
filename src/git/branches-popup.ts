// Branches popup: local and remote branches with a filter; Enter checks
// out, the × deletes a local branch (asking again if it is not merged),
// "New Branch…" creates one from HEAD and checks it out.
import { commands, type GitBranch } from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { bindListKeys, input, Popup } from "../popup/popup";
import { VirtualList } from "../ui/virtual-list";

type Item = { kind: "new" } | { kind: "branch"; branch: GitBranch };

/** Items shown for `query`: New Branch, then local, then remote branches. */
export function branchItems(branches: readonly GitBranch[], query: string): Item[] {
  const q = query.trim().toLowerCase();
  const matches = branches.filter((b) => b.name.toLowerCase().includes(q));
  const order = (b: GitBranch) => (b.current ? 0 : b.remote ? 2 : 1);
  matches.sort((a, b) => order(a) - order(b) || a.name.localeCompare(b.name));
  return [{ kind: "new" }, ...matches.map((branch) => ({ kind: "branch" as const, branch }))];
}

export class BranchesPopup extends Popup {
  private readonly field = input("popup-input", "branches-input", "Filter branches");
  private readonly list: VirtualList<Item>;
  private readonly footer = document.createElement("div");
  private branches: GitBranch[] = [];
  /** Resolves once the branches are listed (Enter waits for it). */
  private loaded: Promise<void> = Promise.resolve();
  /** Creating a branch: the field holds the new name. */
  private creating = false;

  constructor(private readonly notify: (message: string) => void) {
    super("branches", "branches", "Branches");
    const title = document.createElement("div");
    title.className = "popup-title";
    title.textContent = "Branches";
    this.list = new VirtualList<Item>(
      26,
      (item) => this.renderItem(item),
      (item) => {
        void this.activate(item);
      },
      "branch-list",
    );
    this.footer.className = "popup-footer";
    this.footer.dataset.testid = "branches-message";
    this.element.append(title, this.field, this.list.element, this.footer);
    this.field.addEventListener("input", () => {
      if (!this.creating) this.render();
    });
    this.field.addEventListener("keydown", (event) => {
      if (this.creating && event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
        void this.create();
      }
    });
    bindListKeys(this.field, {
      move: (delta) => {
        if (!this.creating) this.list.move(delta);
      },
      activateSelected: () => {
        // Typed quickly, Enter can come before the list: act on the list.
        void this.loaded.then(() => {
          if (!this.creating) this.list.activateSelected();
        });
      },
    });
  }

  override open(): void {
    super.open();
    this.field.focus();
    this.loaded = this.load();
  }

  private async load(): Promise<void> {
    try {
      this.branches = unwrap(await commands.gitBranches());
      this.render();
    } catch (error: unknown) {
      this.footer.textContent = errorMessage(error);
    }
  }

  private render(): void {
    const items = branchItems(this.branches, this.field.value);
    this.list.setItems(items);
    // Preselect the first matching branch rather than "New Branch…".
    if (items.length > 1 && this.field.value.trim() !== "") this.list.move(1);
  }

  private renderItem(item: Item): HTMLElement {
    const row = document.createElement("div");
    row.dataset.testid = "branch-item";
    if (item.kind === "new") {
      row.dataset.name = "";
      row.className = "branch-new";
      row.textContent = "+ New Branch…";
      return row;
    }
    const { branch } = item;
    row.dataset.name = branch.name;
    row.dataset.remote = String(branch.remote);
    row.dataset.current = String(branch.current);
    const name = document.createElement("span");
    name.className = "se-name";
    name.textContent = `${branch.current ? "● " : ""}${branch.name}`;
    const detail = document.createElement("span");
    detail.className = "se-detail";
    detail.textContent = branch.remote ? "remote" : (branch.upstream ?? "local");
    row.append(name, detail);
    if (!branch.remote && !branch.current) {
      const del = document.createElement("button");
      del.type = "button";
      del.className = "branch-delete";
      del.dataset.testid = "branch-delete";
      del.textContent = "×";
      del.setAttribute("aria-label", `Delete ${branch.name}`);
      del.addEventListener("mousedown", (event) => {
        event.preventDefault();
        event.stopPropagation();
        void this.remove(branch.name, false);
      });
      row.append(del);
    }
    return row;
  }

  private async activate(item: Item): Promise<void> {
    if (item.kind === "new") {
      this.creating = true;
      this.field.value = "";
      this.field.placeholder = "New branch name (Enter creates and checks it out)";
      this.field.dataset.testid = "branch-name";
      this.list.setItems([]);
      this.field.focus();
      return;
    }
    if (item.branch.current) {
      this.close();
      return;
    }
    try {
      unwrap(await commands.gitCheckout(item.branch.name, item.branch.remote));
      this.close();
      this.notify(`Checked out ${item.branch.name}.`);
    } catch (error: unknown) {
      this.footer.textContent = errorMessage(error);
    }
  }

  private async create(): Promise<void> {
    const name = this.field.value.trim();
    try {
      unwrap(await commands.gitCreateBranch(name, true));
      this.close();
      this.notify(`Created and checked out ${name}.`);
    } catch (error: unknown) {
      this.footer.textContent = errorMessage(error);
    }
  }

  private async remove(name: string, force: boolean): Promise<void> {
    try {
      const result = unwrap(await commands.gitDeleteBranch(name, force));
      if (result === "notMerged") {
        this.confirmForce(name);
        return;
      }
      this.footer.textContent = `Deleted ${name}.`;
      await this.load();
    } catch (error: unknown) {
      this.footer.textContent = errorMessage(error);
    }
  }

  private confirmForce(name: string): void {
    this.footer.replaceChildren(`${name} is not fully merged; its commits would be lost. `);
    const force = document.createElement("button");
    force.type = "button";
    force.className = "toolbar-button";
    force.dataset.testid = "branch-force-delete";
    force.textContent = "Delete Anyway";
    force.addEventListener("click", () => {
      void this.remove(name, true);
    });
    this.footer.append(force);
  }
}

// Runs in the workbench: the run actions (Shift+F10, Ctrl+Shift+F10,
// Ctrl+F2, Ctrl+F5, Alt+Shift+F10, Alt+4), the toolbar's configuration
// widget, the trust question and the Run tool window. Rust builds the
// commands, starts and ends processes and reads test reports.
import type { DocModel } from "../editor/doc-model";
import type { EditorPane } from "../editor/editor-pane";
import {
  commands,
  type KeyAction,
  type PackageManager,
  type RunConfig,
  type RunFinished,
  type StartRun,
  type TerminalFrame,
  type TerminalSize,
  type TestTarget,
} from "../ipc/bindings";
import { errorMessage, type IpcResult, unwrap } from "../ipc/result";
import type { Popup } from "../popup/popup";
import { emptyConfig } from "./config-form";
import { ConfigChooser, ConfigEditor, TrustDialog } from "./run-popups";
import { RunWindow } from "./run-window";

export const RUN_ACTIONS: ReadonlySet<KeyAction> = new Set<KeyAction>([
  "run",
  "runContext",
  "stopRun",
  "rerun",
  "chooseRunConfiguration",
  "toggleRunWindow",
  "revokeProjectTrust",
]);

export interface RunHost {
  readonly mac: boolean;
  notify(message: string): void;
  openLocation(path: string, line: number, column: number): void;
  activePane(): EditorPane | null;
  docs(): Iterable<DocModel>;
  /** Saves unsaved documents (runs use the files on disk). */
  saveAll(): Promise<void>;
  showPopup(popup: Popup): void;
  /** Shows one bottom tool window and hides the others. */
  showBottom(element: HTMLElement | null): void;
  focusEditor(): void;
}

type Launch = (size: TerminalSize) => Promise<IpcResult<StartRun>>;

/** A test configuration: its run gets the result column from the start. */
function isTest(config: RunConfig): boolean {
  return config.kind.type === "test";
}

export class RunUi {
  readonly window: RunWindow;
  /** Toolbar: the selected configuration, Run and Stop. */
  readonly toolbar = document.createElement("div");
  private readonly chooser = document.createElement("button");
  private readonly runButton = document.createElement("button");
  private readonly stopButton = document.createElement("button");
  private selected: RunConfig | null = null;

  constructor(private readonly host: RunHost) {
    this.window = new RunWindow({
      mac: host.mac,
      notify: (message) => {
        host.notify(message);
      },
      openLocation: (path, line, column) => {
        host.openLocation(path, line, column);
      },
      stop: (id) => {
        void this.stop(id);
      },
      rerun: (id) => {
        void this.launch((size) => commands.rerun(id, size), {
          tests: this.window.isTestRun(id),
        });
      },
      rerunFailed: (id) => {
        void this.launch((size) => commands.rerunFailed(id, size), { tests: true });
      },
      rerunTests: (id, targets: TestTarget[]) => {
        void this.launch((size) => commands.rerunTests(id, targets, size), { tests: true });
      },
      closeRun: (id) => {
        void commands.closeRun(id).then(() => {
          this.updateToolbar();
        });
      },
      onHide: () => {
        host.focusEditor();
      },
    });
    this.toolbar.className = "run-toolbar-main";
    this.chooser.type = "button";
    this.chooser.className = "toolbar-button";
    this.chooser.dataset.testid = "run-config-select";
    this.chooser.addEventListener("click", () => {
      void this.choose();
    });
    this.runButton.type = "button";
    this.runButton.className = "toolbar-button";
    this.runButton.dataset.testid = "run-button";
    this.runButton.textContent = "▶";
    this.runButton.addEventListener("click", () => {
      this.run("run");
    });
    this.stopButton.type = "button";
    this.stopButton.className = "toolbar-button";
    this.stopButton.dataset.testid = "stop-button";
    this.stopButton.textContent = "■";
    this.stopButton.title = "Stop";
    this.stopButton.addEventListener("click", () => {
      this.run("stopRun");
    });
    this.toolbar.append(this.chooser, this.runButton, this.stopButton);
    this.updateToolbar();
  }

  /** Runs a run action. */
  run(action: KeyAction): void {
    switch (action) {
      case "run":
        if (this.selected) {
          const config = this.selected;
          void this.launch((size) => commands.startRun(config, size), { tests: isTest(config) });
        } else {
          void this.choose();
        }
        return;
      case "runContext": {
        const pane = this.host.activePane();
        if (!pane) {
          this.host.notify("Open a file to run.");
          return;
        }
        const state = pane.view.state;
        this.runAt(pane, state.doc.lineAt(state.selection.main.head).number - 1);
        return;
      }
      case "stopRun": {
        const id = this.window.runningId();
        if (id === null) this.host.notify("Nothing is running.");
        else void this.stop(id);
        return;
      }
      case "rerun": {
        const id = this.window.activeId;
        if (id === null) this.run("run");
        else {
          void this.launch((size) => commands.rerun(id, size), {
            tests: this.window.isTestRun(id),
          });
        }
        return;
      }
      case "chooseRunConfiguration":
        void this.choose();
        return;
      case "revokeProjectTrust":
        void this.revokeTrust();
        return;
      case "toggleRunWindow":
        if (this.window.visible && this.window.hasFocus()) {
          this.window.hide();
        } else {
          this.host.showBottom(this.window.element);
          this.window.show();
        }
        return;
      default:
        return;
    }
  }

  /** Runs the test block at zero-based `line` of a pane (gutter, Ctrl+Shift+F10). */
  runAt(pane: EditorPane, line: number | null): void {
    const docId = pane.doc.docId;
    void this.launch((size) => commands.runAt(docId, line, size), { select: true });
  }

  onFrame(frame: TerminalFrame): void {
    this.window.onFrame(frame);
  }

  onFinished(finished: RunFinished): void {
    this.window.onFinished(finished);
    this.updateToolbar();
    this.refreshGutters();
  }

  /** Another project was opened. */
  reset(): void {
    this.window.reset();
    this.selected = null;
    this.updateToolbar();
  }

  private refreshGutters(): void {
    for (const doc of this.host.docs()) doc.scheduleTestBlocks(0);
  }

  private updateToolbar(): void {
    this.chooser.textContent = this.selected?.name ?? "Run…";
    this.chooser.title = "Run configurations (Alt+Shift+F10)";
    this.runButton.title = this.selected ? `Run '${this.selected.name}'` : "Run…";
    this.stopButton.disabled = this.window.runningId() === null;
  }

  private async revokeTrust(): Promise<void> {
    try {
      unwrap(await commands.revokeProjectTrust());
      this.host.notify("Project trust revoked: the next run asks again.");
    } catch (error: unknown) {
      this.host.notify(`Cannot revoke trust: ${errorMessage(error)}`);
    }
  }

  private async stop(id: number): Promise<void> {
    try {
      unwrap(await commands.stopRun(id));
    } catch (error: unknown) {
      this.host.notify(`Stop failed: ${errorMessage(error)}`);
    }
  }

  /**
   * Starts a run: asks Rust, shows the trust question if the project is
   * not trusted yet (then asks again), and gives the run a tab.
   */
  private async launch(
    call: Launch,
    { select = false, tests = false }: { select?: boolean; tests?: boolean } = {},
  ): Promise<void> {
    await this.host.saveAll();
    this.host.showBottom(this.window.element);
    const pending = this.window.prepare(tests);
    let result: StartRun;
    try {
      result = unwrap(await call(pending.size));
    } catch (error: unknown) {
      this.window.discard(pending);
      this.host.notify(`Cannot run: ${errorMessage(error)}`);
      return;
    }
    if (result.type === "needsTrust") {
      this.window.discard(pending);
      const { command, cwd } = result;
      const trusted = await new Promise<boolean>((resolve) => {
        this.host.showPopup(new TrustDialog(command, cwd, resolve));
      });
      if (!trusted) {
        this.host.notify("Not run: the project is not trusted.");
        return;
      }
      try {
        unwrap(await commands.trustProject());
      } catch (error: unknown) {
        this.host.notify(`Cannot trust the project: ${errorMessage(error)}`);
        return;
      }
      await this.launch(call, { select, tests });
      return;
    }
    this.window.adopt(pending, result.run);
    if (select) this.selected = result.run.config;
    this.updateToolbar();
    if (result.run.tests) this.refreshGutters();
  }

  private async choose(): Promise<void> {
    let configs;
    try {
      configs = unwrap(await commands.runConfigurations());
    } catch (error: unknown) {
      this.host.notify(`Run configurations: ${errorMessage(error)}`);
      return;
    }
    const manager = configs.packageManager;
    this.host.showPopup(
      new ConfigChooser(configs, {
        run: (config) => {
          this.selected = config;
          this.updateToolbar();
          void this.launch((size) => commands.startRun(config, size), { tests: isTest(config) });
        },
        edit: (config) => {
          this.host.showPopup(this.editor(config ?? emptyConfig(), manager));
        },
      }),
    );
  }

  private editor(config: RunConfig, manager: PackageManager): ConfigEditor {
    const editor: ConfigEditor = new ConfigEditor(config, manager, {
      save: async (edited) => {
        try {
          const saved = unwrap(await commands.saveRunConfiguration(edited));
          this.selected = saved;
          this.updateToolbar();
          return saved;
        } catch (error: unknown) {
          editor.showError(errorMessage(error));
          return null;
        }
      },
      remove: async (id) => {
        try {
          unwrap(await commands.deleteRunConfiguration(id));
          if (this.selected?.id === id) this.selected = null;
          this.updateToolbar();
          return true;
        } catch (error: unknown) {
          editor.showError(errorMessage(error));
          return false;
        }
      },
      run: (edited) => {
        this.selected = edited;
        this.updateToolbar();
        void this.launch((size) => commands.startRun(edited, size), { tests: isTest(edited) });
      },
    });
    return editor;
  }
}

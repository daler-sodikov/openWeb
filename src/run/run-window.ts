// Run tool window: one tab per run with its output (the M4 terminal view
// over the run's PTY), exit code and duration, Stop and Rerun, and for
// test runs the result tree from Rust with status filters, a summary and
// failure details with clickable stack locations.
import type {
  RunFinished,
  RunInfo,
  TerminalFrame,
  TerminalSize,
  TestLocation,
  TestStatus,
  TestTarget,
  TestTreeNode,
} from "../ipc/bindings";
import { bindListKeys } from "../popup/popup";
import { type TerminalHost, TerminalView } from "../terminal/view";
import { VirtualList } from "../ui/virtual-list";
import {
  exitText,
  formatDuration,
  statusGlyph,
  summaryText,
  type TestRow,
  visibleRows,
} from "./format";

export interface RunWindowHost {
  readonly mac: boolean;
  notify(message: string): void;
  /** Opens a project file at a zero-based line and column. */
  openLocation(path: string, line: number, column: number): void;
  stop(id: number): void;
  rerun(id: number): void;
  rerunFailed(id: number): void;
  rerunTests(id: number, targets: TestTarget[]): void;
  /** A run's tab was closed (the backend ends it if it still runs). */
  closeRun(id: number): void;
  /** The window hid itself. */
  onHide(): void;
}

const ROW_HEIGHT = 22;
const FILTERS: readonly TestStatus[] = ["passed", "failed", "skipped"];

function button(label: string, testid: string, title: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "run-action";
  b.dataset.testid = testid;
  b.textContent = label;
  b.title = title;
  return b;
}

interface Panel {
  info: RunInfo;
  element: HTMLElement;
  view: TerminalView;
  tab: HTMLElement;
  tabStatus: HTMLElement;
  status: HTMLElement;
  stop: HTMLButtonElement;
  rerunFailed: HTMLButtonElement;
  summary: HTMLElement;
  tree: VirtualList<TestRow>;
  details: HTMLElement;
  finished: RunFinished | null;
  shown: Set<TestStatus>;
  collapsed: Set<string>;
  selected: TestTreeNode | null;
}

/** A view made before the run starts, so the PTY gets the right size. */
export interface PendingView {
  holder: HTMLElement;
  view: TerminalView;
  size: TerminalSize;
}

export class RunWindow {
  readonly element = document.createElement("section");
  private readonly tabBar = document.createElement("div");
  private readonly panels = document.createElement("div");
  private readonly empty = document.createElement("div");
  private readonly list: Panel[] = [];
  private active: Panel | null = null;
  /** Frames and ends that arrived before the run's tab existed. */
  private readonly earlyFrames = new Map<number, TerminalFrame[]>();
  private readonly earlyFinished = new Map<number, RunFinished>();
  private readonly terminalHost: TerminalHost;

  constructor(private readonly host: RunWindowHost) {
    this.terminalHost = {
      mac: host.mac,
      notify: (message) => {
        host.notify(message);
      },
      openLink: (target) => {
        if (target.inProject) host.openLocation(target.path, target.line, target.column);
        else host.notify(`${target.path} is outside the project.`);
      },
      newTab: () => undefined,
    };
    this.element.className = "tool-window run-window";
    this.element.dataset.testid = "run-window";
    this.element.setAttribute("aria-label", "Run");
    this.element.hidden = true;
    const header = document.createElement("div");
    header.className = "tool-window-header";
    const title = document.createElement("span");
    title.className = "tool-window-title";
    title.textContent = "Run";
    this.tabBar.className = "terminal-tabs";
    this.tabBar.setAttribute("role", "tablist");
    const start = document.createElement("div");
    start.className = "terminal-header-start";
    start.append(title, this.tabBar);
    const close = document.createElement("button");
    close.type = "button";
    close.className = "tool-window-close";
    close.textContent = "×";
    close.setAttribute("aria-label", "Hide Run");
    close.addEventListener("click", () => {
      this.hide();
    });
    header.append(start, close);
    this.panels.className = "run-panels";
    this.empty.className = "tool-window-empty";
    this.empty.dataset.testid = "run-empty";
    this.empty.textContent = "Nothing has run yet.";
    this.element.append(header, this.empty, this.panels);
  }

  get visible(): boolean {
    return !this.element.hidden;
  }

  hasFocus(): boolean {
    return this.element.contains(document.activeElement);
  }

  show(): void {
    this.element.hidden = false;
    this.empty.hidden = this.list.length > 0;
    this.active?.view.focus();
  }

  hide(): void {
    this.element.hidden = true;
    this.host.onHide();
  }

  /** The selected run's id, if any. */
  get activeId(): number | null {
    return this.active?.info.id ?? null;
  }

  /** The selected run if it is still running, else the newest running one. */
  runningId(): number | null {
    if (this.active && !this.active.finished) return this.active.info.id;
    const running = this.list.filter((p) => !p.finished);
    return running[running.length - 1]?.info.id ?? null;
  }

  /** Forgets every run (the backend closed them with the old project). */
  reset(): void {
    for (const panel of this.list) panel.view.dispose();
    this.list.length = 0;
    this.active = null;
    this.earlyFrames.clear();
    this.earlyFinished.clear();
    this.tabBar.replaceChildren();
    this.panels.replaceChildren();
    this.element.hidden = true;
  }

  /** Shows the window with a fresh output view, sized, for a run to start. */
  prepare(): PendingView {
    this.element.hidden = false;
    this.empty.hidden = true;
    const view = new TerminalView(this.terminalHost);
    const holder = document.createElement("div");
    holder.className = "run-panel";
    const body = document.createElement("div");
    body.className = "run-body";
    body.append(view.element);
    holder.append(body);
    for (const panel of this.list) panel.element.hidden = true;
    this.panels.append(holder);
    const size = view.measure() ?? { cols: 120, rows: 20 };
    return { holder, view, size };
  }

  /** The run did not start: drop the prepared view. */
  discard(pending: PendingView): void {
    pending.view.dispose();
    pending.holder.remove();
    if (this.active) this.select(this.active);
    else if (this.list.length === 0) this.element.hidden = true;
  }

  /** A run started in the prepared view: give it a tab. */
  adopt(pending: PendingView, info: RunInfo): void {
    const panel = this.createPanel(info, pending);
    pending.view.attach(info.id, pending.size);
    for (const frame of this.earlyFrames.get(info.id) ?? []) panel.view.applyFrame(frame);
    this.earlyFrames.delete(info.id);
    this.select(panel);
    const finished = this.earlyFinished.get(info.id);
    this.earlyFinished.delete(info.id);
    if (finished) this.onFinished(finished);
  }

  onFrame(frame: TerminalFrame): void {
    const panel = this.list.find((p) => p.info.id === frame.id);
    if (panel) {
      panel.view.applyFrame(frame);
      return;
    }
    const early = this.earlyFrames.get(frame.id) ?? [];
    early.push(frame);
    this.earlyFrames.set(frame.id, early);
  }

  onFinished(finished: RunFinished): void {
    const panel = this.list.find((p) => p.info.id === finished.id);
    if (!panel) {
      this.earlyFinished.set(finished.id, finished);
      return;
    }
    panel.finished = finished;
    const code = finished.exitCode ?? null;
    panel.view.showExit(code);
    panel.status.textContent = exitText(code, finished.durationMs, finished.stopped);
    panel.status.dataset.state = code === 0 || finished.stopped ? "ok" : "failed";
    panel.stop.disabled = true;
    const state = finished.stopped ? "stopped" : code === 0 ? "ok" : "failed";
    panel.tab.dataset.state = state;
    panel.tabStatus.textContent = state === "stopped" ? "■" : state === "ok" ? "✓" : "✗";
    if (!panel.info.tests) return;
    if (finished.counts) {
      panel.summary.textContent = summaryText(finished.counts);
      panel.summary.dataset.state = finished.counts[1] > 0 ? "failed" : "ok";
      panel.rerunFailed.disabled = finished.counts[1] === 0;
    } else {
      panel.summary.textContent = finished.reportError ?? "No test results.";
      panel.summary.dataset.state = "failed";
    }
    panel.selected = firstFailed(finished.tests ?? []);
    this.renderTree(panel);
    this.renderDetails(panel);
  }

  private createPanel(info: RunInfo, pending: PendingView): Panel {
    const element = pending.holder;
    element.className = info.tests ? "run-panel run-tests-panel" : "run-panel";
    element.dataset.testid = "run-panel";
    element.dataset.id = String(info.id);

    const toolbar = document.createElement("div");
    toolbar.className = "run-toolbar";
    const rerun = button("⟳ Rerun", "run-rerun", "Rerun");
    const stop = button("■ Stop", "run-stop", "Stop");
    const rerunFailed = button("Rerun Failed", "run-rerun-failed", "Rerun failed tests");
    rerunFailed.disabled = true;
    rerunFailed.hidden = !info.tests;
    const status = document.createElement("span");
    status.className = "run-status";
    status.dataset.testid = "run-status";
    status.textContent = "Running…";
    toolbar.append(rerun, stop, rerunFailed);

    const command = document.createElement("div");
    command.className = "run-command";
    command.dataset.testid = "run-command";
    command.textContent = info.command;

    const summary = document.createElement("div");
    summary.className = "run-summary";
    summary.dataset.testid = "run-summary";
    summary.textContent = "Running tests…";
    summary.hidden = !info.tests;

    const tests = document.createElement("div");
    tests.className = "run-tests";
    tests.hidden = !info.tests;
    const details = document.createElement("div");
    details.className = "run-details";
    details.dataset.testid = "run-details";

    const tab = document.createElement("div");
    tab.className = "terminal-tab run-tab";
    tab.dataset.testid = "run-tab";
    tab.dataset.id = String(info.id);
    tab.setAttribute("role", "tab");
    const tabStatus = document.createElement("span");
    tabStatus.className = "run-tab-status";
    tabStatus.textContent = statusGlyph("running");
    const label = document.createElement("span");
    label.className = "terminal-tab-title";
    label.textContent = info.name;
    const close = document.createElement("button");
    close.type = "button";
    close.className = "terminal-tab-close";
    close.dataset.testid = "run-tab-close";
    close.textContent = "×";
    close.setAttribute("aria-label", `Close ${info.name}`);
    tab.append(tabStatus, label, close);

    // Filled in below; the list's callbacks run only after that.
    const ref: { panel: Panel | null } = { panel: null };
    const tree = new VirtualList<TestRow>(
      ROW_HEIGHT,
      (row) => (ref.panel ? this.renderRow(ref.panel, row) : document.createElement("div")),
      (row) => {
        const panel = ref.panel;
        if (!panel) return;
        panel.selected = row.node;
        this.renderTree(panel, true);
        this.renderDetails(panel);
      },
      "run-test-tree",
    );
    tree.element.tabIndex = 0;
    bindListKeys(tree.element, tree);

    const filters = document.createElement("div");
    filters.className = "run-filters";
    filters.hidden = !info.tests;
    const shown = new Set<TestStatus>(FILTERS);
    for (const filter of FILTERS) {
      const toggle = button(
        `${statusGlyph(filter)} ${filter.charAt(0).toUpperCase()}${filter.slice(1)}`,
        `run-filter-${filter}`,
        `Show ${filter} tests`,
      );
      toggle.setAttribute("aria-pressed", "true");
      toggle.addEventListener("click", () => {
        if (shown.has(filter)) shown.delete(filter);
        else shown.add(filter);
        toggle.setAttribute("aria-pressed", String(shown.has(filter)));
        if (ref.panel) this.renderTree(ref.panel);
      });
      filters.append(toggle);
    }
    toolbar.append(filters, status);

    tests.append(tree.element, details);
    const body = pending.view.element.parentElement ?? document.createElement("div");
    body.prepend(tests);
    element.replaceChildren(toolbar, command, summary, body);

    const panel: Panel = {
      info,
      element,
      view: pending.view,
      tab,
      tabStatus,
      status,
      stop,
      rerunFailed,
      summary,
      tree,
      details,
      finished: null,
      shown,
      collapsed: new Set(),
      selected: null,
    };
    ref.panel = panel;
    rerun.addEventListener("click", () => {
      this.host.rerun(info.id);
    });
    stop.addEventListener("click", () => {
      this.host.stop(info.id);
    });
    rerunFailed.addEventListener("click", () => {
      this.host.rerunFailed(info.id);
    });
    tab.addEventListener("mousedown", (event) => {
      if (event.target === close) return;
      event.preventDefault();
      this.select(panel);
    });
    close.addEventListener("click", () => {
      this.close(panel);
    });
    this.list.push(panel);
    this.tabBar.append(tab);
    return panel;
  }

  private select(panel: Panel): void {
    this.active = panel;
    for (const p of this.list) {
      const selected = p === panel;
      p.element.hidden = !selected;
      p.tab.setAttribute("aria-selected", String(selected));
    }
    this.empty.hidden = true;
    panel.view.focus();
  }

  private close(panel: Panel): void {
    const index = this.list.indexOf(panel);
    if (index < 0) return;
    this.list.splice(index, 1);
    panel.view.dispose();
    panel.element.remove();
    panel.tab.remove();
    this.host.closeRun(panel.info.id);
    const next = this.list[Math.min(index, this.list.length - 1)];
    if (next) {
      this.select(next);
    } else {
      this.active = null;
      this.empty.hidden = false;
    }
  }

  private renderTree(panel: Panel, keepScroll = false): void {
    const rows = visibleRows(panel.finished?.tests ?? [], panel.shown, panel.collapsed);
    const scroll = panel.tree.element.scrollTop;
    panel.tree.setItems(rows);
    if (keepScroll) panel.tree.element.scrollTop = scroll;
  }

  private renderRow(panel: Panel, row: TestRow): HTMLElement {
    const node = row.node;
    const el = document.createElement("div");
    el.className = `run-test-row test-${node.status}`;
    if (panel.selected === node) el.classList.add("current");
    el.dataset.testid = "test-row";
    el.dataset.status = node.status;
    el.dataset.kind = node.kind;
    el.dataset.name = node.kind === "file" ? node.name : node.fullName.join(" > ");
    el.style.setProperty("--depth", String(row.depth));
    const toggle = document.createElement("span");
    toggle.className = "run-test-toggle";
    toggle.textContent = row.hasChildren ? (row.collapsed ? "▸" : "▾") : "";
    toggle.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (row.collapsed) panel.collapsed.delete(row.key);
      else panel.collapsed.add(row.key);
      this.renderTree(panel, true);
    });
    const glyph = document.createElement("span");
    glyph.className = "run-test-status";
    glyph.textContent = statusGlyph(node.status);
    const name = document.createElement("span");
    name.className = "run-test-name";
    name.textContent = node.name;
    const duration = document.createElement("span");
    duration.className = "run-test-duration";
    duration.textContent = formatDuration(node.durationMs);
    el.append(toggle, glyph, name, duration);
    el.addEventListener("dblclick", () => {
      this.openTest(node);
    });
    return el;
  }

  private openTest(node: TestTreeNode): void {
    const location = node.location;
    if (!location.inProject) {
      this.host.notify(`${location.path} is outside the project.`);
      return;
    }
    this.host.openLocation(location.path, Math.max(0, location.line - 1), 0);
  }

  private renderDetails(panel: Panel): void {
    const node = panel.selected;
    if (!node) {
      panel.details.replaceChildren();
      return;
    }
    const head = document.createElement("div");
    head.className = "run-details-head";
    const title = document.createElement("span");
    title.className = `run-details-title test-${node.status}`;
    title.textContent = `${statusGlyph(node.status)} ${node.name}`;
    const again = button("Run", "run-test-again", "Run this again");
    again.addEventListener("click", () => {
      this.host.rerunTests(panel.info.id, [{ path: node.location.path, name: node.fullName }]);
    });
    const open = button("Go to", "run-test-open", "Open in the editor");
    open.disabled = !node.location.inProject;
    open.addEventListener("click", () => {
      this.openTest(node);
    });
    head.append(title, again, open);
    const parts: HTMLElement[] = [head];
    for (const failure of collectFailures(node)) {
      const message = document.createElement("pre");
      message.className = "run-failure";
      message.dataset.testid = "run-failure";
      message.textContent = failure.message;
      parts.push(message);
      for (const frame of failure.frames) parts.push(this.frameLink(frame));
    }
    panel.details.replaceChildren(...parts);
  }

  private frameLink(frame: TestLocation): HTMLElement {
    const link = document.createElement("button");
    link.type = "button";
    link.className = "run-frame";
    link.dataset.testid = "run-frame";
    link.textContent = `${frame.path}:${String(frame.line)}:${String(frame.column)}`;
    link.disabled = !frame.inProject;
    link.addEventListener("click", () => {
      this.host.openLocation(
        frame.path,
        Math.max(0, frame.line - 1),
        Math.max(0, frame.column - 1),
      );
    });
    return link;
  }
}

/** The node's failures, or those of the tests below it. */
function collectFailures(node: TestTreeNode): TestTreeNode["failures"] {
  if (node.failures.length > 0 || node.kind === "test") return node.failures;
  return node.children.flatMap((c) => collectFailures(c));
}

function firstFailed(nodes: readonly TestTreeNode[]): TestTreeNode | null {
  for (const node of nodes) {
    if (node.kind === "test" && node.status === "failed") return node;
    const found = firstFailed(node.children);
    if (found) return found;
  }
  return null;
}

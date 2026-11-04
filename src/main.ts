import "./styles.css";

import { renderFatal, renderedAppName } from "./app/shell";
import { Workbench } from "./app/workbench";
import { PROTOCOL_VERSION, commands } from "./ipc/bindings";
import { unwrap } from "./ipc/result";
import { initTypingTrace } from "./perf/typing-trace";

async function main(root: HTMLElement): Promise<void> {
  const info = await commands.appInfo();
  if (info.protocolVersion !== PROTOCOL_VERSION) {
    renderFatal(
      root,
      `Backend protocol ${String(info.protocolVersion)} does not match frontend protocol ${String(PROTOCOL_VERSION)}.`,
    );
    return;
  }
  document.title = info.name;
  await initTypingTrace();
  const workbench = new Workbench(root, info, await commands.keymap());

  const report = await commands.reportFrontendReady({
    renderedAppName: renderedAppName(root),
    documentTitle: document.title,
    protocolVersion: PROTOCOL_VERSION,
  });
  if (report.status === "error") {
    renderFatal(root, report.error.message);
    return;
  }

  // `?project=<path>` is used by the UI tests (the browser has no folder
  // dialog); the desktop app passes the command-line folder instead.
  const fromUrl = new URLSearchParams(window.location.search).get("project");
  const initial = fromUrl
    ? { folder: fromUrl, file: null }
    : unwrap(await commands.initialProject());
  if (initial) {
    await workbench.openProject(initial.folder);
    if (initial.file) await workbench.openFile(initial.file);
  }
}

const root = document.getElementById("app");
if (!root) {
  throw new Error("index.html is missing the #app root element");
}
main(root).catch((error: unknown) => {
  renderFatal(root, `Failed to start: ${String(error)}`);
});

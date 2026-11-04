import "./styles.css";

import { renderFatal, renderShell, renderedAppName } from "./app/shell";
import { PROTOCOL_VERSION, commands } from "./ipc/bindings";

async function main(root: HTMLElement): Promise<void> {
  const info = await commands.appInfo();
  if (info.protocolVersion !== PROTOCOL_VERSION) {
    renderFatal(
      root,
      `Backend protocol ${String(info.protocolVersion)} does not match frontend protocol ${String(PROTOCOL_VERSION)}.`,
    );
    return;
  }
  renderShell(root, info);

  const result = await commands.reportFrontendReady({
    renderedAppName: renderedAppName(root),
    documentTitle: document.title,
    protocolVersion: PROTOCOL_VERSION,
  });
  if (result.status === "error") {
    renderFatal(root, result.error.message);
  }
}

const root = document.getElementById("app");
if (!root) {
  throw new Error("index.html is missing the #app root element");
}
main(root).catch((error: unknown) => {
  renderFatal(root, `Failed to start: ${String(error)}`);
});

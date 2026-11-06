import { describe, expect, it, vi } from "vitest";

// The window's only IPC call in this test: the last commit message, held
// until the test releases it.
let release: (message: string) => void = () => undefined;
vi.mock("../ipc/bindings", () => ({
  commands: {
    gitLastCommitMessage: () =>
      new Promise((resolve) => {
        release = (message) => {
          resolve({ status: "ok", data: message });
        };
      }),
  },
}));

const { CommitWindow } = await import("./commit-window");

function setUp() {
  const window = new CommitWindow({
    showDiff: () => undefined,
    openFile: () => undefined,
    notify: () => undefined,
  });
  document.body.append(window.element);
  const amend = window.element.querySelector<HTMLInputElement>('[data-testid="commit-amend"]');
  const message = window.element.querySelector<HTMLTextAreaElement>(
    '[data-testid="commit-message"]',
  );
  if (!amend || !message) throw new Error("commit window is missing its fields");
  const checkAmend = () => {
    amend.checked = true;
    amend.dispatchEvent(new Event("change"));
  };
  return { amend, message, checkAmend };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("Amend", () => {
  it("fills in the last message when the box is empty", async () => {
    const { message, checkAmend } = setUp();
    checkAmend();
    release("Last message");
    await settle();
    expect(message.value).toBe("Last message");
  });

  it("never overwrites text typed while the last message loads", async () => {
    const { message, checkAmend } = setUp();
    checkAmend();
    message.value = "Typed meanwhile";
    release("Last message");
    await settle();
    expect(message.value).toBe("Typed meanwhile");
  });

  it("does not fill in after Amend was unchecked again", async () => {
    const { amend, message, checkAmend } = setUp();
    checkAmend();
    amend.checked = false;
    release("Last message");
    await settle();
    expect(message.value).toBe("");
  });
});

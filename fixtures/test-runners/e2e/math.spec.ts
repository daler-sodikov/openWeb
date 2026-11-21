import { expect, test } from "@playwright/test";

test.describe("e2e math", () => {
  test("adds without a browser", () => {
    expect(1 + 1).toBe(2);
  });

  test("fails without a browser", () => {
    // Fails on purpose: the fixture needs a failing test.
    expect(2 * 2).toBe(5);
  });
});

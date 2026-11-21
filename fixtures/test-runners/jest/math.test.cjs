const { add, divide } = require("./math");

describe("math", () => {
  it("adds", () => {
    expect(add(1, 2)).toBe(3);
  });

  it("divides", () => {
    // Fails on purpose: the fixture needs a failing test.
    expect(divide(1, 4)).toBe(0.5);
  });

  it.skip("is skipped", () => {
    expect(true).toBe(false);
  });

  describe("tables", () => {
    it.each([
      [1, 1, 2],
      [2, 3, 5],
    ])("adds %i and %i", (a, b, sum) => {
      expect(add(a, b)).toBe(sum);
    });
  });
});

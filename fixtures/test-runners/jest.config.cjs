// Jest runs only the CommonJS tests in jest/. Vitest (src/) and Playwright
// (e2e/) have their own folders, so no runner picks up another's tests.
module.exports = {
  testEnvironment: "node",
  testMatch: ["<rootDir>/jest/**/*.test.cjs"],
};

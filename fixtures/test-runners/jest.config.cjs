// Jest runs only the CommonJS tests in jest/. Vitest (src/) and Playwright
// (e2e/) have their own folders, so no runner picks up another's tests.
//
// The folder is in `roots`, not in `testMatch`: Jest puts `<rootDir>` into
// the glob, and on Windows leaves the backslash before a folder that starts
// with a dot (a temp folder such as `.tmpAbC`) unconverted, which the glob
// reads as an escaped dot, so nothing matches ("No tests found").
module.exports = {
  testEnvironment: "node",
  roots: ["<rootDir>/jest"],
  testMatch: ["**/*.test.cjs"],
};

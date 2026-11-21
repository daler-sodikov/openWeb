# test-runners fixture

A small project for the run configuration and test runner tests (M5a).

- `src/math.test.ts` (Vitest): one test fails on purpose, one is skipped,
  one is parameterized.
- `e2e/math.spec.ts` (Playwright): needs no browser; one test fails on
  purpose.
- `jest/math.test.cjs` (Jest, `jest.config.cjs`): the same five tests as the
  Vitest file, in CommonJS so Jest needs no transform. Each runner only
  matches its own folder, so none picks up another's tests.
- `npm run start` (`src/main.js`) prints a file location and exits;
  `npm run dev` (`src/serve.js`) runs until stopped.

It has no committed `node_modules`. Tests that run the real tools copy the
project and link a `node_modules` into the copy, or skip with a message:

- Vitest and Playwright come from the repository's `node_modules` (installed
  there for the frontend: `bun install`).
- Jest comes from this folder: `bun install --cwd fixtures/test-runners`
  (`bun.lock` is committed). The tests do not copy `bun.lock`, so the copy's
  package manager stays npm.

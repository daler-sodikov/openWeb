# test-runners fixture

A small project for the run configuration and test runner tests (M5a).

- `src/math.test.ts` (Vitest): one test fails on purpose, one is skipped,
  one is parameterized.
- `e2e/math.spec.ts` (Playwright): needs no browser; one test fails on
  purpose.
- `npm run start` (`src/main.js`) prints a file location and exits;
  `npm run dev` (`src/serve.js`) runs until stopped.

It has no `node_modules`. Tests that run the real tools copy the project and
link the repository's `node_modules` into the copy (Vitest and Playwright are
installed there for the frontend), or skip with a message.

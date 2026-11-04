# Test fixtures

Sample projects used by integration and end-to-end tests. Tests copy a
fixture into a temporary directory before editing it, so the files here are
never modified by a test run.

- `react-ts/`: React + TypeScript (M1, Playwright `ui` tests)
- planned: `next/` (M3), `vue/` (phase 4), `monorepo/` (M2/M3)

Fixtures are small, original sample code. Do not copy third-party projects in.
These files are test data: they are excluded from ESLint and Prettier.

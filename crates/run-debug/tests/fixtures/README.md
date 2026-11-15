# Reporter fixtures

- `vitest.json`: recorded from Vitest 5.1.0
  (`vitest run --reporter=default --reporter=json --outputFile.json=...`) on
  `fixtures/test-runners`.
- `playwright.json`: recorded from Playwright 1.64.1
  (`playwright test --reporter=list,json`, `PLAYWRIGHT_JSON_OUTPUT_NAME`) on
  `fixtures/test-runners`.
- `jest.handwritten.json`: **not recorded** (Jest is not installed here):
  written from Jest's documented aggregated-result format (`--json
  --testLocationInResults`) to mirror the same tests, plus a file that fails
  to load.

Machine-specific paths are replaced: `__ROOT__` is the fixture project,
`__REPO__` the repository, `__NODE__` the Node executable.

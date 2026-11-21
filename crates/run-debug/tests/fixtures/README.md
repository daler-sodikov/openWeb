# Reporter fixtures

- `vitest.json`: recorded from Vitest 5.1.0
  (`vitest run --reporter=default --reporter=json --outputFile.json=...`) on
  `fixtures/test-runners`.
- `playwright.json`: recorded from Playwright 1.64.1
  (`playwright test --reporter=list,json`, `PLAYWRIGHT_JSON_OUTPUT_NAME`) on
  `fixtures/test-runners`.
- `jest.json`: recorded from Jest 30.5.2
  (`jest --json --testLocationInResults --outputFile=...`) on
  `fixtures/test-runners` (`jest/math.test.cjs`).
- `jest.handwritten.json`: **not recorded**: written from Jest's documented
  aggregated-result format to mirror the Vitest tests, plus a file that fails
  to load (a case the real fixture does not have). The fake run backend
  replays it.

Machine-specific paths are replaced: `__ROOT__` is the fixture project,
`__REPO__` the repository, `__NODE__` the Node executable.

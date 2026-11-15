# test-runners fixture

A small project for the run configuration and test runner tests (M5a).
`src/math.test.ts` has one test that fails on purpose, one skipped test and a
parameterized one; `e2e/math.spec.ts` is a Playwright spec that needs no
browser. It has no `node_modules`: the tools are found in the repository's
`node_modules` (Node resolution walks up), or the real-run tests skip.

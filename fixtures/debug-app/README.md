# debug-app

A small TypeScript program for the debugger tests (M5b). `dist/app.js` and
`dist/app.js.map` are real `tsc` output, committed so tests need no compiler:

```bash
cd fixtures/debug-app
../../node_modules/.bin/tsc -p .   # tsc 6.0.3
node dist/app.js                   # prints "total 14"
```

Lines (zero-based) of `src/app.ts` used by tests: line 9 `sum += item.price`
(generated line 4), line 11 is blank and has no code (a breakpoint there moves
to line 12, `return sum;`). Regenerate only with the same `tsconfig.json`, and
update the expected positions in `crates/run-debug/src/sourcemap.rs`.

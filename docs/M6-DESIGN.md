# Milestone 6: UI design

Status: NOT STARTED. Do not begin until the maintainer says "start M6".
Until then, build features functionally; do not polish visuals beyond what a
feature needs to be usable. New UI should use CSS variables where that is easy,
so M6 has less to migrate.

Order of work: M4 (terminal, Git basics) -> M5 (run/debug, test runner) -> M6
(this document). After M6, re-measure typing latency (see Part 3).

## Why

The UI works but looks unfinished: clipped labels at the left window edge
("errule", "ROJECT"), an empty box instead of an icon at the right end of the
tab bar, no icons, plain tabs/toolbar/tree, plain popups. No design rules
existed before this milestone.

## Reference

Reference screenshots of a professional IDE live OUTSIDE the repo, in
`~/Desktop/design-refs/` (never copy them into the repo or docs/media):

1. Open File dialog
2. Main window: project tree and empty editor
3. Menu bar with File menu open over an editor with code
4. Settings dialog
5. Git menu over an empty editor with a hint list

Use them for layout, density, spacing and interaction feel ONLY.

Hard rules:

- No JetBrains/WebStorm/IntelliJ/"Islands" names or product logos anywhere
  in the product, UI, window titles or binaries. README keeps "Not
  affiliated with JetBrains."
- Never take assets or theme files from an installed IDE. Assets may come
  only from public repositories, under their stated licenses (see Icons).
- Do not copy the reference color values; use the Ferrule palette below.
- Fonts bundled locally (CSP forbids remote), with license texts: Inter (UI)
  and JetBrains Mono (OFL) or another permissive monospace (editor).
- Record every license in THIRD_PARTY_LICENSES.md.

## Icons (decision: hybrid)

Approach modeled on github.com/l0ng-ai/openidea (see its
THIRD_PARTY_LICENSES.md): file-type icons from JetBrains' open-source icon
set, everything else our own or from a permissive library.

1. **UI icons** (toolbar, chevrons, search, settings, split, close, tool
   window stripes, status bar, menus): Lucide (`lucide-static`, ISC), inline
   SVG, 16px, `currentColor`, colors from tokens.
2. **File-type icons** (project tree, tabs, Search Everywhere results): may
   be taken from `JetBrains/intellij-community`, `platform/icons/src/expui`
   (Apache-2.0), plus the other open repos OpenIDEA lists when their icon
   exists there and its license allows it (for example intellij-rust, MIT).
   Monochrome icons are recolored to `currentColor` at render time.
3. **Everything else** (languages with no open-source JetBrains icon, such as
   Go, Ruby, Dart, Lua, Zig, Svelte, dotenv, lock, license): draw our own in
   the same style (16px, simple strokes), kept in `src/ui/ferrule/`.

Mandatory license procedure BEFORE copying any JetBrains icon:

- Open the upstream repository's LICENSE and NOTICE at a pinned commit and
  confirm the license of that exact path. Plugin directories may carry their
  own license (the open-source plugin repos state this per plugin). If a
  path's license is unclear or not permissive, do not copy it; draw our own.
  Do not rely on this document or on the OpenIDEA repo for the legal answer.
- Keep each file's original copyright header.
- Mark every modified file as modified (Apache-2.0 section 4).
- Record in THIRD_PARTY_LICENSES.md and NOTICE: upstream repo, pinned
  commit hash, exact paths used, copyright line, and the full license text
  (Apache-2.0 and any MIT text) once.
- Copy only the icons we actually use (a short allowlist in
  `src/ui/icons.allowlist.json`), never whole folders.
- Trademarks are not licensed: no product logos (WebStorm, IntelliJ, the
  JetBrains logo), no product names in icon filenames or UI.
- A test fails if an icon file is present that is not on the allowlist, or
  if an allowlisted file lacks its license header or an entry in
  THIRD_PARTY_LICENSES.md.

## Color tokens (dark theme, final values)

All colors are CSS custom properties in `src/styles/tokens.css`. No hardcoded
colors or sizes in components. Derive the light theme with the same structure
and WCAG AA contrast.

```
frame_bg:       #222428
panel_bg:       #17181b
editor_bg:      #17181b
editor_line_bg: #1d1e22
border:         #2e3036
text:           #d4d6dc
text_muted:     #9aa0ab
text_faint:     #4d525b
selection_bg:   #34406b
hover_bg:       #2d3036
accent:         #e8a33d   (Ferrule amber: primary buttons, focus ring, active tab underline)
on_accent:      #1a1204
link:           #5fa0f0
error:          #ec7a72
warning:        #e3b341
success:        #5fb878
popup_bg:       #2a2c31
popup_border:   #3a3d44
```

Layering rule: frame > popup > panel/editor, each step slightly lighter than
the one below; the current-line highlight is one small step above the editor.
Colors for file-type icons that we draw ourselves (fallbacks when no
open-source icon exists): JS yellow, TS blue, JSON purple, MD slate, CSS teal,
HTML orange, config gray. Icons taken from the open-source set keep their own
colors or follow `currentColor`; check every one against both themes. Keep
the existing syntax highlighting colors (they are original); only check
their contrast against the new editor_bg.

## Part 1: shell and components

1. Tokens (dark primary, light equally polished), 4px spacing scale, radii,
   shadows, fonts. Shared primitives: button, icon button, tab, tree row, list
   row, popup/menu, tooltip, panel header, status bar item, input, dialog
   (title, content, OK/Cancel/Apply, focus trap, Esc/Enter).
2. "Islands" layout: dark frame, rounded panels with gaps between them. Left
   stripe with tool window icons (Project, Find, Problems, and later Terminal,
   Git), bottom stripe. Panels resizable, sizes persisted, minimum window size
   enforced. Fix the clipped text at the left window edge.
3. Top bar: main menu, project name with a small colored initials badge,
   search icon (opens Search Everywhere), settings icon. Items for features
   that do not exist are NOT shown. No fake functionality anywhere.
4. Project tree: root row with muted path, chevrons, folder and file-type
   icons, indent guides, selected/hover states, greyed ignored entries,
   "library root" label next to node_modules.
5. Editor chrome: tabs with file icon, close button on hover, modified dot,
   overflow handling, split button with a real icon; gutter with fold
   chevrons, current-line highlight, indent guides; top-right inspection
   widget with error/warning counts from the LSP; scrollbar with diagnostic
   markers.
6. Status bar: breadcrumbs on the left (project > file > scope, replacing the
   separate breadcrumb bar); on the right line:col, line separator, encoding,
   indent, TypeScript server state.
7. Empty-editor state: centered hint list, shortcuts right-aligned in a
   brighter color, using the real shortcuts.
8. Menu bar and menus (File, Edit, View, Navigate, Code, Refactor, Run, Tools,
   Git, Window, Help) with submenus, separators, right-aligned shortcuts and
   mnemonics. Only actions that really exist are enabled; unimplemented ones
   are hidden or disabled with a clear label.
9. Migrate ALL popups (Search Everywhere, Go to, Find/Replace, hover and
   diagnostics, completion, signature help, rename, usages, Alt+Enter,
   chooser, reload bar, terminal and Git panels from M4) to the shared style.
10. Theme switch (system / dark / light).

## Part 2 (only after the maintainer says "continue with M6 part 2")

Settings dialog (Appearance: theme, UI zoom, fonts; Editor: font, size;
Keymap read-only list; TypeScript server path; real settings only),
notification toasts with actions (server missing, external file change),
welcome screen with recent projects.

## Part 3: performance after the redesign

Typing latency was 37-53 ms median on Xvfb at the end of M3b (budget 8 ms),
with Rust and highlighting under 0.2 ms, so the cost is in WebKit painting,
not in Rust or IPC. After the redesign, re-measure it on a real display if
possible. Profile first (CSS cost, layout thrash, paint areas, caret blink),
then fix the biggest items and update perf/RESULTS.md with before/after.

## Verification (mandatory)

The real WebKit window cannot be captured under Xvfb (screenshots are black).
Verify in Chromium with Playwright against the headless backend: screenshot
every key screen in dark and light, open each image, compare against the
references, list the differences and fix them. Add a Playwright visual check
that fails on clipped or overflowing text. Ferrule's own screenshots go in
`docs/media/`.

## Rules

Do not change behavior or IPC beyond what is listed. Small Conventional
Commits, keep all checks green, do not push. At the end report what you
looked at, what you changed because of what you saw, and what still looks
weak.

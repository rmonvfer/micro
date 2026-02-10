# ohm TUI parity specification

The goal is verbatim: **100% parity with the ohm TUI**. This document is the reference for
that work. It describes ohm's interface exactly enough that it can be rebuilt in
`crates/micro-tui` without opening ohm's source.

## Sources and confidence

Everything here was read from two trees:

- `/Users/ramon/code/biz/ohm` — the reference. `packages/coding-agent/src/modes/interactive/`
  is the interface, `packages/tui/src/` is the toolkit underneath it, and
  `packages/coding-agent/src/core/tools/` holds the per-tool renderers that decide what a
  tool execution looks like.
- `/Users/ramon/code/biz/micro/crates/micro-tui/` — the target.

A third tree, `/Users/ramon/code/biz/agent47/agent47-tui/`, is a Kotlin port that was
deliberately made to match ohm. Where it agrees with ohm, confidence is high. Where it
differs, ohm wins and the difference is called out.

Values quoted as hex, glyphs, key names, and integers are transcribed from source. Anything
inferred rather than read is marked **(inferred)**.

A note on names: ohm's `APP_NAME` defaults to `"ohm"` and `APP_TITLE` to `"Ω"`; both are
overridable by config. Substitute `micro` wherever a literal app name appears.

---

## 1. Gap inventory

### 1.1 The one structural difference that governs everything else

ohm is **not** a fixed-viewport application. `packages/tui/src/tui.ts` renders the entire
document — header, every message ever sent, the editor, the footer — into one array of lines
and writes it to the terminal with differential updates. Content that grows past the terminal
height flows into the terminal's own scrollback. There is no internal scroll model, no
scroll offset for the transcript, and no `PageUp` handler for history: the terminal scrolls,
the mouse wheel scrolls, and the user's terminal search works on the transcript.

micro is the opposite: `render/mod.rs` lays out a fixed frame, `scroll.rs` owns a scroll
offset, and the transcript is clipped to `transcript_area.height`.

This is the single largest gap and it is a prerequisite for the rest. In ratatui the
equivalent is `Viewport::Inline` plus `Terminal::insert_before` — finished transcript blocks
get pushed into scrollback, and only the live tail (streaming message, editor, footer) is
redrawn. Marked in §9 as the first item, and as partially reproducible.

### 1.2 Component-by-component

Columns: what ohm has, where it lives, where micro's equivalent lives, and the verdict.

| Area | ohm | micro | Verdict |
|---|---|---|---|
| Render model | `packages/tui/src/tui.ts` (append + diff into scrollback) | `src/render/mod.rs`, `src/scroll.rs` (fixed viewport) | **Different.** §1.1 |
| Theme | `theme/dark.json`, `theme/light.json`, `theme/theme.ts` — 50 named tokens, JSON-loadable, user themes, live file watch, truecolor→256 downsampling | `src/theme.rs` — 10 fields, two hardcoded palettes, `MICRO_THEME` env var | **Different.** micro has ~20% of the token set. §3 |
| Horizontal padding | Per component (`paddingX`, usually 1); borders and background bands run edge to edge | Global `PADDING = 1` inset in `render/mod.rs` | **Close.** Text insets match; see §4.1 |
| Startup header | `interactive-mode.ts:764` `ExpandableText` — logo, keybinding hints, onboarding, collapsible with `ctrl+o` | `render/mod.rs:164` `intro()` — 4 lines, only while transcript is empty | **Different.** ohm's header is permanent and expandable |
| User message | `components/user-message.ts` — full-width `userMessageBg` band, markdown body | `render/transcript.rs:77` `push_user` — `› ` prefix, `muted` text, no band | **Different** |
| Assistant message | `components/assistant-message.ts` — markdown, no background, leading blank line | `render/transcript.rs:117` `push_markdown` | **Close**; markdown fidelity differs (§5.2) |
| Thinking block | `assistant-message.ts:104` — full italic markdown in `thinkingText`, or one static `Thinking...` label when hidden | `render/transcript.rs:90` `push_thinking` — `✻ ` marker, collapsed to latest line | **Different.** ohm has no `✻`, and its collapsed form is a fixed label, not a live tail |
| Tool execution | `components/tool-execution.ts` + per-tool `renderCall`/`renderResult` in `core/tools/*.ts` — background-banded box, colour = pending/success/error | `src/tools.rs` + `render/tool.rs` — `⏺ ` marker line, no band | **Different** |
| Bash (user `!`) | `components/bash-execution.ts` — horizontal rules above/below, `$ cmd` header, live streaming output, 20-line preview | none | **Missing in micro** |
| Diff | `components/diff.ts` + `core/tools/edit-diff.ts` — `+NNN`/`-NNN`/` NNN` with line numbers, word-level inverse highlighting, `...` elision | `src/diff.rs`, `render/tool.rs` `Row::Added/Removed/Context/Divider` — no line numbers, no intra-line highlight | **Different** |
| Compaction summary | `components/compaction-summary-message.ts` | none | **Missing in micro** |
| Branch summary | `components/branch-summary-message.ts` | none | **Missing in micro** |
| Skill invocation | `components/skill-invocation-message.ts` | none | **Missing in micro** |
| Custom message / entry | `components/custom-message.ts`, `custom-entry.ts` (extension-rendered) | none | **Missing in micro**; only needed if micro grows extensions |
| Notice lines | ad hoc `Text(theme.fg("dim"/"warning"/"error", …), 1, 0)` from `interactive-mode.ts` | `render/transcript.rs:150` `push_notice` with `· `/`⟳ `/`✗ ` markers | **Different.** ohm uses no marker glyph |
| Editor | `packages/tui/src/components/editor.ts` (2333 lines) — 21 bindings, kill ring, undo, history, paste markers, jump-to-char, sticky column, autocomplete | `src/editor.rs` (642 lines) + `src/event.rs` | **Large gap.** §6 |
| Editor frame | Horizontal rule above and below, colour encodes thinking level or bash mode; scroll indicators in the rules | Tinted band, `surface` background, placeholder text | **Different** |
| Autocomplete | `packages/tui/src/autocomplete.ts`, `fuzzy.ts`, `SelectList` — slash commands, `@` file fuzzy search via `fd`, path completion, `#` and extension triggers | `src/menu.rs`, `src/fuzzy.rs` — slash commands only | **Partial.** `fuzzy.rs` is a faithful port; §2 |
| Selectors/dialogs | 14 components: model, session, theme, settings, thinking, login, oauth, tree, trust, config, scoped-models, extension selector/input/editor, first-time-setup, user-message-selector, show-images | `src/approval.rs` + `render/approval.rs` only | **Missing in micro** |
| Approval prompt | none (ohm uses per-tool policy without an in-TUI prompt of this shape) | `render/approval.rs` | **micro-only.** Keep it; style it to ohm's dialog pattern (§7.1) |
| Status indicator | `components/status-indicator.ts` — 4 kinds (working/retry/compaction/branch-summary), braille spinner @ 80 ms, always 2 rows | `render/status.rs` `activity_line` — 1 row, only while running | **Different.** §8 |
| Footer | `components/footer.ts` — 2–3 lines: cwd+branch+session, then usage stats + right-aligned model | `render/status.rs` `status_line` — 1 line, `status` background tint | **Different.** §8 |
| Keybinding hints | `components/keybinding-hints.ts` — `dim(keys) + muted(" description")`, `alt`→`option` on macOS | none | **Missing in micro** |
| Countdown timer | `components/countdown-timer.ts` | none | **Missing in micro** |
| Images | `packages/tui/src/terminal-image.ts` — kitty/iterm inline images, sixel fallback | none | **Missing in micro.** Low priority |
| OSC 133 shell zones | `user-message.ts:53`, `assistant-message.ts:78` | none | **Missing in micro.** Trivial to add |
| Hyperlinks | OSC 8 on file paths and markdown links | none | **Missing in micro** |
| Focus / result picking | none — ohm expands *all* tool output at once with `ctrl+o` | `ctrl+↑/↓` picks a result, `ctrl+o` toggles it | **micro-only.** ohm's `ctrl+o` is global. §9 flags this as a deliberate decision point |
| Mouse wheel | terminal-native (ohm never leaves the normal buffer) | `event.rs` `WHEEL_LINES = 3` | **micro-only**, obsolete once §1.1 lands |

### 1.3 Where agent47 disagrees with ohm

Recorded because these are places a previous porter drifted.

- `agent47-tui/src/main/kotlin/.../theme/Theme.kt` sets `muted = #9e9e9e` and `dim = #7a7a7a`.
  ohm's dark theme is `muted = #808080` (`gray`) and `dim = #666666` (`dimGray`). **ohm wins.**
  micro's `theme.rs` copied agent47's values (`dim` = rgb(122,122,122) = `#7a7a7a`,
  `muted` = rgb(158,158,158) = `#9e9e9e`) — both are wrong against ohm.
- agent47 caps the editor at 8 rows (`layout/TuiLayout.kt` `min(8, …)`); ohm uses
  `max(5, floor(terminalRows * 0.3))`. **ohm wins.**
- agent47 comments `statusBarBg` as "ohm footer has no background fill" — correct, and micro's
  `theme.status` tint on the footer is a deviation.
- agent47's `accent = #8abeb7`, `success = #b5bd68`, `error = #cc6666`, `warning = #ffff00`,
  `userMessageBg = #343541` all match ohm exactly. micro's `warning` (rgb 222,174,96) does not.

---

## 2. Autocomplete and the slash-command menu

Read first if you are wiring slash commands. Sources: `packages/tui/src/autocomplete.ts`,
`packages/tui/src/fuzzy.ts`, `packages/tui/src/components/select-list.ts`, and the
autocomplete half of `packages/tui/src/components/editor.ts`.

### 2.1 When the menu opens

The editor holds one `AutocompleteProvider`. Suggestions are requested from
`Editor.insertCharacter`, `Editor.handleBackspace`, `Editor.handleForwardDelete`,
`Editor.moveCursor` (when a menu is already open), and Tab.

Triggers, in the order the editor checks them (`editor.ts:1112`):

1. **`/` at the start of a message.** `isAtStartOfMessage()` is true when the cursor is on
   logical line 0 *and* the text before the cursor trims to `""` or `"/"`. The slash menu is
   never offered on line 2 or below (`isSlashMenuAllowed()` — `cursorLine === 0`).
2. **A trigger character at a token boundary.** Default triggers are `["@", "#"]`; a provider
   may add more single characters (never `/`, never whitespace). Fires when the character is
   the first on the line, or the character before it is a space or a tab.
3. **A word character inside an existing context.** `[a-zA-Z0-9.\-_]` re-queries when either
   `isInSlashCommandContext(textBeforeCursor)` (line 0, and the text before the cursor
   left-trims to something starting with `/`) or the trigger pattern
   `(?:^|[\s])[@#…][^\s]*$` matches.
4. **Tab** (`tui.input.tab`). On line 0 with a `/` prefix and no space yet, this is a
   slash-command completion; otherwise it is a forced file completion.

Debounce: 0 ms in the normal case. 20 ms (`ATTACHMENT_AUTOCOMPLETE_DEBOUNCE_MS`) when the text
before the cursor matches the attachment pattern
`(?:^|[ \t])(?:@(?:"[^"]*|[^\s]*)|[<other triggers>][^\s]*)$`, and never for an explicit Tab
or a forced request. Requests are serialised, given an `AbortController`, and discarded if the
buffer text or cursor moved while they were in flight.

### 2.2 What the provider returns

`CombinedAutocompleteProvider.getSuggestions(lines, cursorLine, cursorCol, {signal, force})`
resolves in this order:

1. **`@` prefix** → fuzzy file search. Requires `fd` on PATH. Runs
   `fd --base-directory <dir> --max-results 100 --type f --type d --follow --hidden
   --exclude .git --exclude '.git/*' --exclude '.git/**' [--full-path] <pattern>`.
   `--full-path` is added when the query contains `/`. Results are scored (§2.4), sorted
   descending, and the top **20** are returned. Item shape:
   `{ value: "@path" or "@\"path with spaces\"", label: basename + ("/" if dir),
   description: full display path }`.
2. **`/` prefix with no space** → slash commands, filtered by `fuzzyFilter` on the command
   name. Item shape: `{ value: name, label: name, description: hint + " — " + description }`
   where `hint` is the command's `argumentHint`. If there is no hint, the description alone;
   if neither, `undefined`.
3. **`/name ` with a space** → that command's `getArgumentCompletions(argumentText)`, if it
   has one. The prefix for replacement is the argument text, not the whole line.
4. **A path-like token** → directory listing completion. Triggers naturally when the token
   contains `/`, starts with `.`, starts with `~/`, or is empty immediately after a space.
   Under a forced Tab it triggers on any token. Entries are filtered case-insensitively by
   `startsWith`, directories sort before files, then `localeCompare` by label.

Returning `null` or an empty item list closes the menu.

### 2.3 How candidates rank

`fuzzy.ts` — a **penalty** score, lower is better. `fuzzyFilter(items, query, getText)` splits
the query on `[\s/]+`; **every** token must match, and the item's total is the sum of the token
scores. An empty/whitespace query returns the input list unchanged, in its original order.

`fuzzyMatch(query, text)` lowercases both, then walks `text` matching `query` characters in
order:

| Condition | Score change |
|---|---|
| Match adjacent to the previous match (run length `n`, starting at 1) | `-= n * 5` |
| Match not adjacent, gap of `g` characters | `+= g * 2`, and the run counter resets |
| Match at a word boundary (index 0, or preceded by `[\s\-_./:]`) | `-= 10` |
| Every match, at text index `i` | `+= i * 0.1` |
| Query equals the whole text | `-= 100` |
| Not all query characters consumed | no match |

Fallback: if the query fails and it is `letters+digits` or `digits+letters`, the halves are
swapped and retried; a swapped match scores `+5` on top of its own score so an as-typed match
always wins.

`crates/micro-tui/src/fuzzy.rs` already implements this correctly, including the swap and the
`WORD_SEPARATORS` set. No change needed there.

Note the `@`-file path uses a *different*, simpler scorer (`scoreEntry`, `autocomplete.ts:697`),
not `fuzzyMatch`:

| Condition on the basename | Score |
|---|---|
| Exact match (case-insensitive) | 100 |
| Basename starts with query | 80 |
| Query is a substring of the basename | 50 |
| Query is a substring of the full path | 30 |
| otherwise | 0 → filtered out |
| Entry is a directory and score > 0 | `+10` |

Higher is better here. Ties keep `fd`'s order.

### 2.4 Initial selection

`getBestAutocompleteMatchIndex(items, prefix)` (`editor.ts:2096`) runs after the list is built:

1. An item whose `value` **equals** the prefix, case-sensitively → selected.
2. Otherwise the first item whose `value` **starts with** the prefix.
3. Otherwise index 0 is left as-is.

`crates/micro-tui/src/menu.rs` always starts at 0. That is a divergence; add the pre-selection.

### 2.5 How the menu draws

The menu is rendered by `SelectList` and appended by the editor **below the editor's bottom
border rule**, each row left-padded by the editor's `paddingX` and right-padded to the content
width.

Constants (`select-list.ts`):

- `maxVisible` = the editor's `autocompleteMaxVisible`, default **5**, clamped to `[3, 20]`.
- Selected row prefix `"→ "`; unselected `"  "`. Both 2 columns.
- Primary (name) column: for the slash menu, `minPrimaryColumnWidth = 12`,
  `maxPrimaryColumnWidth = 32`. The actual width is `clamp(widest label + 2, min, max)`.
  `PRIMARY_COLUMN_GAP = 2`.
- The description column is drawn only when the item has one **and** the render width is
  `> 40` **and** the remaining width after prefix + name + gap is `> 10`
  (`MIN_DESCRIPTION_WIDTH`). Descriptions are collapsed to a single line
  (`/[\r\n]+/ → " "`, then trimmed) and truncated with no ellipsis.
- Scroll window: `start = clamp(selected - floor(maxVisible/2), 0, len - maxVisible)`.
  Selection stays near the middle. micro's `Menu::window` already matches.
- When the window does not cover everything, one extra row is appended:
  `"  (<selected+1>/<total>)"`, truncated to `width - 2`, in the `scrollInfo` colour.
- Empty result set renders `"  No matching commands"` in the `noMatch` colour. In practice the
  editor closes the menu instead of showing this, so it is rarely seen.

Colours (`theme.ts:1267` `getSelectListTheme`):

| Role | Token | Dark value |
|---|---|---|
| `selectedPrefix` | `accent` | `#8abeb7` |
| `selectedText` (the whole selected row, name **and** description) | `accent` | `#8abeb7` |
| `description` (unselected rows) | `muted` | `#808080` |
| `scrollInfo` | `muted` | `#808080` |
| `noMatch` | `muted` | `#808080` |

Unselected rows draw the name in the terminal's default foreground (no colour applied) and the
description in `muted`. The selected row is `accent` end to end.

### 2.6 Keys while the menu is open

Handled in `editor.ts:653`, before any editing key:

| Key | Action |
|---|---|
| `tui.select.cancel` (`escape`, `ctrl+c`) | Close the menu. The keystroke is consumed |
| `tui.select.up` (`up`) | Move selection up, wrapping to the last item |
| `tui.select.down` (`down`) | Move selection down, wrapping to the first item |
| `tui.input.tab` (`tab`) | Commit the selection, close the menu, **do not** submit |
| `tui.select.confirm` (`enter`) | Commit the selection, close the menu, and — **only when the prefix starts with `/`** — fall through to submit the message |

That last row matters and micro does not do it: in ohm, pressing Enter on a highlighted slash
command runs it immediately. For any other completion kind (`@`, file path, command argument)
Enter commits and stops.

Anything else falls through to normal editing, and the editor re-queries afterwards.

### 2.7 How a completion is applied

`CombinedAutocompleteProvider.applyCompletion` replaces `prefix.length` characters ending at
the cursor:

- **Slash command** (prefix starts with `/`, nothing but whitespace before it, and no second
  `/` in the prefix): line becomes `<before>/<value> <after>`, cursor at
  `before.length + value.length + 2`.
- **`@` attachment**: line becomes `<before><value><suffix><after>` where `suffix` is `" "` for
  a file and `""` for a directory (so the user keeps completing into it). If the value ends in
  a closing quote and the item is a directory, the cursor lands one column left of the quote.
- **Command argument** (the text before the cursor contains both `/` and a space): value is
  inserted with no trailing space.
- **Path**: same as command argument.

A quoted prefix (`"…` or `@"…`) whose completion supplies a closing quote will eat an existing
`"` immediately after the cursor rather than doubling it.

Special case: a **forced** Tab completion that yields exactly one item applies it immediately
without ever showing a menu (`editor.ts:2253`).

---

## 3. Theme tokens

ohm themes are JSON, validated against `theme/theme-schema.json`, and loaded from
`theme/dark.json`, `theme/light.json`, plus any user file in the custom themes directory. A
theme has an optional `vars` block of reusable colours and a required `colors` block of 50
semantic tokens (49 required + optional `thinkingMax`, which falls back to `thinkingXhigh`).

A token value is one of: a `#RRGGBB` hex string, the name of an entry in `vars`, an integer
`0–255` (a 256-colour palette index), or `""` meaning "the terminal's default". Empty string
emits `ESC[39m` for foreground and `ESC[49m` for background.

`theme.fg(token, text)` emits `<fg-ansi><text>ESC[39m` — it resets **only** the foreground.
`theme.bg(token, text)` emits `<bg-ansi><text>ESC[49m` — resets only the background. This is
what lets a background band survive foreground colour changes inside it.

When the terminal lacks truecolor, hex values are downsampled by `rgbTo256`: the 6×6×6 cube
(channel stops `0, 95, 135, 175, 215, 255`) versus the 24-step grey ramp (`8 + i*10`), compared
with a luminance-weighted distance (`0.299 R² + 0.587 G² + 0.114 B²`); the grey ramp only wins
when max−min channel spread is `< 10`.

### 3.1 The dark theme, verbatim

`vars`:

| Name | Value |
|---|---|
| `cyan` | `#00d7ff` |
| `blue` | `#5f87ff` |
| `green` | `#b5bd68` |
| `red` | `#cc6666` |
| `yellow` | `#ffff00` |
| `text` | `#d4d4d4` |
| `gray` | `#808080` |
| `dimGray` | `#666666` |
| `darkGray` | `#505050` |
| `accent` | `#8abeb7` |
| `selectedBg` | `#3a3a4a` |
| `userMsgBg` | `#343541` |
| `toolPendingBg` | `#282832` |
| `toolSuccessBg` | `#283228` |
| `toolErrorBg` | `#3c2828` |
| `customMsgBg` | `#2d2838` |

`colors`, resolved, with the light theme beside it and what each paints:

| Token | Dark | Light | Paints |
|---|---|---|---|
| `accent` | `#8abeb7` | `#5a8080` | Logo, selected list rows, list cursor `→ `, dialog titles, file paths in tool headers, streaming caret |
| `border` | `#5f87ff` | `#547da7` | `DynamicBorder` default — dialog and bordered-loader rules |
| `borderAccent` | `#00d7ff` | `#5a8080` | Highlighted borders (session selector rules) |
| `borderMuted` | `#505050` | `#b0b0b0` | The editor's own rules when no thinking/bash colour applies |
| `success` | `#b5bd68` | `#588458` | Success states, `✓` checkmarks in selectors |
| `error` | `#cc6666` | `#aa5555` | Error text, aborted/failed messages |
| `warning` | `#ffff00` | `#9a7326` | Warnings, truncation notices, `(cancelled)`, read line ranges |
| `muted` | `#808080` | `#6c6c6c` | Secondary text: hints' description half, list descriptions, `... N more lines`, loader message |
| `dim` | `#666666` | `#767676` | Very dim: footer, key names in hints, status lines, `(ctrl+o to expand)` |
| `text` | `#d4d4d4` | `#1f2328` | Default text |
| `thinkingText` | `#808080` | `#6c6c6c` | Thinking block body (always italic) |
| `selectedBg` | `#3a3a4a` | `#d0d0e0` | Selected-item background (background token) |
| `userMessageBg` | `#343541` | `#e8e8e8` | The band behind a user message (background token) |
| `userMessageText` | `#d4d4d4` | `#1f2328` | User message body text |
| `customMessageBg` | `#2d2838` | `#ede7f6` | Band behind custom/compaction/branch/skill blocks (background token) |
| `customMessageText` | `#d4d4d4` | `#1f2328` | Body of those blocks |
| `customMessageLabel` | `#9575cd` | `#7e57c2` | The `[compaction]`, `[branch]`, `[skill]`, `[<type>]` labels |
| `toolPendingBg` | `#282832` | `#e8e8f0` | Tool box while running (background token) |
| `toolSuccessBg` | `#283228` | `#e8f0e8` | Tool box on success (background token) |
| `toolErrorBg` | `#3c2828` | `#f0e8e8` | Tool box on error (background token) |
| `toolTitle` | `#d4d4d4` | `#1f2328` | Tool name in the header — plain text colour, made bold |
| `toolOutput` | `#808080` | `#6c6c6c` | Tool output body |
| `mdHeading` | `#f0c674` | `#9a7326` | Markdown headings |
| `mdLink` | `#81a2be` | `#547da7` | Markdown link text |
| `mdLinkUrl` | `#666666` | `#767676` | The ` (url)` shown after a link when the terminal has no OSC 8 |
| `mdCode` | `#8abeb7` | `#5a8080` | Inline code spans |
| `mdCodeBlock` | `#b5bd68` | `#588458` | Code block body when there is no syntax highlighting |
| `mdCodeBlockBorder` | `#808080` | `#6c6c6c` | The ``` ``` ``` fence lines |
| `mdQuote` | `#808080` | `#6c6c6c` | Blockquote text (always italic) |
| `mdQuoteBorder` | `#808080` | `#6c6c6c` | The `│ ` blockquote gutter |
| `mdHr` | `#808080` | `#6c6c6c` | Horizontal rules |
| `mdListBullet` | `#8abeb7` | `#588458` | List bullets and numbers |
| `toolDiffAdded` | `#b5bd68` | `#588458` | `+` diff lines |
| `toolDiffRemoved` | `#cc6666` | `#aa5555` | `-` diff lines |
| `toolDiffContext` | `#808080` | `#6c6c6c` | unchanged diff lines |
| `syntaxComment` | `#6A9955` | `#008000` | Syntax: comments |
| `syntaxKeyword` | `#569CD6` | `#0000FF` | Syntax: keywords |
| `syntaxFunction` | `#DCDCAA` | `#795E26` | Syntax: function names |
| `syntaxVariable` | `#9CDCFE` | `#001080` | Syntax: variables |
| `syntaxString` | `#CE9178` | `#A31515` | Syntax: strings |
| `syntaxNumber` | `#B5CEA8` | `#098658` | Syntax: numbers |
| `syntaxType` | `#4EC9B0` | `#267F99` | Syntax: types |
| `syntaxOperator` | `#D4D4D4` | `#000000` | Syntax: operators |
| `syntaxPunctuation` | `#D4D4D4` | `#000000` | Syntax: punctuation |
| `thinkingOff` | `#505050` | `#b0b0b0` | Editor rule colour, thinking level `off` |
| `thinkingMinimal` | `#6e6e6e` | `#767676` | Editor rule, `minimal` |
| `thinkingLow` | `#5f87af` | `#547da7` | Editor rule, `low` |
| `thinkingMedium` | `#81a2be` | `#5a8080` | Editor rule, `medium` |
| `thinkingHigh` | `#b294bb` | `#875f87` | Editor rule, `high` |
| `thinkingXhigh` | `#d183e8` | `#8b008b` | Editor rule, `xhigh` |
| `thinkingMax` | `#ff5fff` | `#af005f` | Editor rule, `max` (optional; falls back to `thinkingXhigh`) |
| `bashMode` | `#b5bd68` | `#588458` | Editor rule and `$ cmd` header while the prompt starts with `!` |

The six background tokens are `selectedBg`, `userMessageBg`, `customMessageBg`,
`toolPendingBg`, `toolSuccessBg`, `toolErrorBg`. Everything else is a foreground.

`export.pageBg` / `export.cardBg` / `export.infoBg` (`#18181e` / `#1e1e24` / `#3c3728` dark;
`#f8f8f8` / `#ffffff` / `#fffae6` light) are for HTML transcript export only and do not affect
the TUI.

### 3.2 What micro must change

`crates/micro-tui/src/theme.rs` should become a 50-token struct loaded from JSON with these
exact defaults. Minimum viable step: keep the struct but fix the values that are simply wrong —
`dim` → `#666666`, `muted` → `#808080`, `warning` → `#ffff00`, and add the background tokens.
`Color::Reset` for body text is fine only if `text` is `""`; ohm's dark theme sets it to
`#d4d4d4`, so body text is explicitly coloured.

---

## 4. Layout geometry

### 4.1 Vertical order

`interactive-mode.ts:699` adds these containers to the root, in this order. Each renders zero
or more lines; the concatenation is the document.

1. `headerContainer` — startup banner
2. `loadedResourcesContainer` — extensions, skills, themes loaded at startup
3. `chatContainer` — the transcript
4. `pendingMessagesContainer` — queued follow-ups and in-flight bash blocks
5. `statusContainer` — the working/retry/compaction indicator, or `IdleStatus`
6. `widgetContainerAbove` — extension widgets (a single `Spacer(1)` when empty)
7. `editorContainer` — the editor, or whichever dialog has replaced it
8. `widgetContainerBelow` — extension widgets (nothing when empty)
9. `footer`

There is **no** global horizontal inset. Every component receives the full terminal width and
applies its own `paddingX`. In practice text components use `paddingX = 1`, so body text sits
one column in from each edge, while rules and background bands run edge to edge.

micro's `render/mod.rs` insets everything by `PADDING = 1` including the tinted regions. The
text result is the same; the difference shows on bands and rules, which in ohm reach column 0
and the last column.

### 4.2 The two primitives that produce all the spacing

**`Box(paddingX, paddingY, bgFn)`** — `packages/tui/src/components/box.ts`:

```
contentWidth = max(1, width - paddingX * 2)
for each child: render at contentWidth, prefix every line with paddingX spaces
emit paddingY blank rows
emit the child rows
emit paddingY blank rows
every emitted row is padded with spaces to exactly `width`, then bgFn is applied to the whole row
```

A `Box` with no children renders **zero** lines.

**`Text(text, paddingX, paddingY, bgFn?)`** — `packages/tui/src/components/text.ts`:

```
if text is empty or all whitespace -> zero lines
tabs -> 3 spaces
contentWidth = max(1, width - paddingX * 2)
wrap with wrapTextWithAnsi (ANSI-aware, does not count escapes toward width)
each line becomes: paddingX spaces + line + paddingX spaces, then padded to `width`
paddingY blank rows above and below
```

`Spacer(n)` emits `n` empty strings — genuinely empty, not width-padded, so they inherit the
terminal background.

`DynamicBorder(colorFn)` emits exactly one line: `colorFn("─".repeat(max(1, width)))`.

### 4.3 The editor frame

`editor.ts:464`. Default `paddingX` is **0** (setting `editorPaddingX`).

```
maxPadding   = floor((width - 1) / 2)
paddingX     = min(configured, maxPadding)
contentWidth = max(1, width - paddingX * 2)
layoutWidth  = max(1, contentWidth - (paddingX > 0 ? 0 : 1))   // reserve a column for the cursor
```

Rendered as:

1. **Top rule.** `"─" * width` in `borderColor`. If the editor is scrolled down, the rule is
   replaced by `"─── ↑ <n> more "` followed by `─` filling the rest of the width.
2. **Visible rows.** `maxVisibleLines = max(5, floor(terminalRows * 0.3))`. The scroll offset
   is nudged only enough to keep the cursor row visible. Each row is
   `paddingX spaces + text + pad-to-contentWidth + paddingX spaces`.
3. **Bottom rule.** `"─" * width`, or `"─── ↓ <n> more "` + fill when content continues below.
4. **The autocomplete menu**, if open — see §2.5.

The cursor is drawn as reverse video on the grapheme under it (`ESC[7m<g>ESC[0m`), or a reverse
video space when the cursor is past the end of the line. A zero-width marker
(`\x1b_pi:c\x07`) is emitted just before it so the real hardware cursor can be placed there for
IME candidate windows.

Border colour: `theme.getBashModeBorderColor()` (i.e. `bashMode`) when the prompt's trimmed
text starts with `!`, otherwise `theme.getThinkingBorderColor(level)` — one of the seven
`thinking*` tokens. This is ohm's most distinctive small touch: **the rules around the input
change colour with the reasoning budget.**

### 4.4 The startup state

Before any message, with `quietStartup` off, the screen is:

```
<blank>
<logo line>
<hint lines>
<blank>
<onboarding line>
<blank>
──────────────────────────────────────────────  (editor top rule)
<empty editor row with the reverse-video cursor>
──────────────────────────────────────────────  (editor bottom rule)
<blank>                                          (IdleStatus row 1)
<blank>                                          (IdleStatus row 2)
<cwd (branch)>
<stats ...                          model • thinking level>
```

Header text, `paddingX = 1`, `paddingY = 0`, wrapped in `Spacer(1)` above and below.

Collapsed form (the default; toggled by `ctrl+o`, the same key as tool expansion):

```
<bold accent "ohm"><dim " v0.x.y">
<hint> · <hint> · <hint> · <hint> · <hint>
<dim "Press ctrl+o to show full startup help and loaded resources.">

<dim "ohm can explain its own features and look up its docs. Ask it how to use or extend ohm.">
```

The five compact hints, joined by `muted(" · ")`:
`escape interrupt`, `ctrl+c/ctrl+d clear/exit`, `/ commands`, `! bash`, `ctrl+o more`.

Expanded form replaces the middle with 19 hints, one per line:

```
escape to interrupt
ctrl+c to clear
ctrl+c twice to exit
ctrl+d to exit (empty)
ctrl+z to suspend
ctrl+k to delete to end
shift+tab to cycle thinking level
ctrl+p/shift+ctrl+p to cycle models
ctrl+l to select model
ctrl+o to expand tools
ctrl+t to expand thinking
ctrl+g for external editor
/ for commands
! to run bash
!! to run bash (no context)
alt+enter to queue follow-up
alt+up to edit all queued messages
ctrl+v to paste image (with text fallback)
drop files to attach
```

Each hint is `dim(keys) + muted(" " + description)` (§8.4). On macOS `alt` is displayed as
`option`.

---

## 5. Per-message-type rendering

Every entry below is what `chatContainer` accumulates. Separation between entries is by
explicit `Spacer(1)`, added by the caller in `interactive-mode.ts:3144 addMessageToChat`.

### 5.1 User message

`components/user-message.ts`.

- Preceded by `Spacer(1)` when the transcript is not empty.
- A `Box(outputPad, 1, bg = userMessageBg)` containing
  `Markdown(text, 0, 0, markdownTheme, { color: userMessageText },
  { preserveOrderedListMarkers: true, preserveBackslashEscapes: true })`.
- `outputPad` defaults to **1** (setting `outputPad`, only 0 or 1).
- So: one blank row tinted `#343541`, then the body rows tinted `#343541` with a 1-column
  inset, then one blank tinted row. The tint spans the full terminal width.
- No prefix glyph, no author label, no border.
- The first rendered line is prefixed with OSC 133 `A` (`\x1b]133;A\x07`) and the last with
  `B` then `C`, marking a shell prompt zone so terminals can jump between prompts.

micro currently draws `› ` + `muted` text and no band. Replacing that with the band is the
highest-visibility single change in the whole document.

### 5.2 Assistant message

`components/assistant-message.ts`. No background at any point.

- If the message has any non-empty text or thinking content, a `Spacer(1)` is emitted first.
- Content blocks render **in order**:
  - `text` → `Markdown(text.trim(), outputPad, 0, markdownTheme)`, default styling.
  - a run of consecutive `thinking` blocks → joined with `\n\n` and rendered as one
    `Markdown(..., outputPad, 0, markdownTheme, { color: thinkingText, italic: true })`.
    A `Spacer(1)` follows the run **only** when more visible content comes after it — which is
    what avoids a stray blank line before a tool block.
  - When thinking is hidden (`ctrl+t` / setting `hideThinkingBlock`), the whole run collapses
    to a single `Text(italic(fg(thinkingText, "Thinking...")), outputPad, 0)`. Not the latest
    line — a fixed label. Extensions may change the label.
- Failure tails, each preceded by `Spacer(1)` and drawn in `error`:
  - `stopReason === "length"` →
    `"Error: Model stopped because it reached the maximum output token limit. The response may be incomplete."`
  - `stopReason === "aborted"` and no tool calls → the message's `errorMessage`, or
    `"Operation aborted"`.
  - `stopReason === "error"` and no tool calls → `"Error: <errorMessage or 'Unknown error'>"`.
- OSC 133 zone markers are added exactly as for user messages, but **only when the message has
  no tool calls**.

Markdown rendering (`packages/tui/src/components/markdown.ts`) — the pieces that matter:

| Element | Rendering |
|---|---|
| Heading, level 1 | `heading(bold(underline(text)))`, no `#` prefix |
| Heading, level 2 | `heading(bold(text))`, no `#` prefix |
| Heading, level ≥ 3 | `heading(bold("### "))` prefix **is** kept, then the styled text |
| After any heading | one blank line, unless the next token is a `space` token |
| Paragraph | inline-rendered; one blank line after, unless followed by a list or a `space` token |
| Fenced code | fence line ```` ```lang ```` in `mdCodeBlockBorder`; each body line prefixed by `codeBlockIndent` (default two spaces) and syntax-highlighted if the language is recognised, else `mdCodeBlock`; closing ```` ``` ```` fence; then a blank line |
| Inline code | `mdCode` |
| Bold / italic / strikethrough / underline | terminal SGR via chalk |
| Link | `mdLink(underline(text))`. With OSC 8 support the URL is attached invisibly; without it, ` (url)` in `mdLinkUrl` is appended when the text differs from the href |
| Blockquote | every line gets a `mdQuoteBorder("│ ")` gutter; content is `mdQuote(italic(...))`; content width is `width - 2`; trailing blank quote lines are trimmed; one blank line after |
| Horizontal rule | `mdHr("─" * min(width, 80))`, then a blank line |
| Unordered list | `mdListBullet("- ")` by default; with `preserveOrderedListMarkers` the source marker (`-`, `+`, `*`) is kept |
| Ordered list | `mdListBullet("N. ")` counting from the list's `start`; with `preserveOrderedListMarkers` the source marker (`N.` or `N)`) is kept |
| Task list item | marker + `"[x] "` or `"[ ] "` |
| Nested list | four spaces per depth level |
| List continuation lines | indented to the marker's visible width |
| Loose list | blank line between items, except after the last |
| Table | box-drawing frame `┌─┬─┐ │ ├─┼─┤ └─┴─┘`, bold header row, per-column width solving with a 30-column cap on unbroken words, separator row between every data row, falls back to raw markdown when narrower than `3n + 1 + n` columns |
| Tabs | replaced by three spaces before parsing |

Streaming detail worth copying: a partially-arrived closing code fence is trimmed off so the
block does not shrink and flicker when the final backtick lands
(`markdown.ts:25 trimPartialClosingFences`).

micro's `markdown.rs` covers headings, code fences, quotes, bullets, and `**`/`__`/backtick
inline. It lacks: heading levels behaving differently, `*`/`_` single-marker emphasis, links,
strikethrough, tables, task lists, nested lists, ordered-list numbering from `start`, syntax
highlighting, and the fence lines themselves (micro drops the fence and tints the body; ohm
prints the fences and does **not** tint).

### 5.3 Tool execution

`components/tool-execution.ts`. Two shells.

**Default shell** — the component is:

```
Spacer(1)
Box(1, 1, bg)   containing: <call component>, then <result component> once the result arrives
```

`bg` is chosen live:

| State | Token | Dark |
|---|---|---|
| still streaming arguments or executing (`isPartial`) | `toolPendingBg` | `#282832` |
| finished, `isError` | `toolErrorBg` | `#3c2828` |
| finished, success | `toolSuccessBg` | `#283228` |

So a tool block is a full-width tinted card: one blank tinted row, the header, the body, one
blank tinted row. The `Box` inset is 1 column, and the call/result components themselves use
`paddingX = 0`, so tool text sits at column 1.

**Self shell** (`renderShell: "self"`, used by `edit`) — the component supplies its own `Box`,
and `ToolExecutionComponent.render` emits one bare `""` line before it instead of the `Spacer`.
Visually identical.

The generic fallback, used when no tool definition exists, is a `Text(…, 1, 1, bg)` containing
`bold(fg(toolTitle, name))`, then a blank line, then `JSON.stringify(args, null, 2)`, then the
text output.

Per-tool headers and bodies (`packages/coding-agent/src/core/tools/*.ts`). All headers are one
line; `path` renders as `accent(shortenPath(p))` wrapped in an OSC 8 `file://` hyperlink when
the terminal supports it, `toolOutput("...")` while the argument is still streaming, and
`error("[invalid arg]")` when the argument is the wrong type.

| Tool | Header | Collapsed body | Expanded body |
|---|---|---|---|
| `read` | `bold("read") <path>` + `warning(":<start>[-<end>]")` when `offset`/`limit` are set | **nothing at all** — a successful collapsed read is a bare header line. On error, the first 10 lines are shown | blank line, then the whole file content syntax-highlighted by extension, then `muted("... (N more lines,") + hint + muted(")")` if anything is still hidden |
| `read` of a `SKILL.md` | `customMessageLabel(bold("[skill]")) + customMessageText(<skill dir name>) + dim(" (ctrl+o to expand)")` | — | falls back to the normal `read` header |
| `read` of ohm's own docs/README/examples | `bold("read docs") accent(<relative path>) dim(" (ctrl+o to expand)")` | — | — |
| `write` | `bold("write") <path>` | blank line, blank line, then the first 10 lines of content, highlighted; then `muted("... (N more lines, T total,") + hint + muted(")")` | all lines |
| `edit` | `bold("edit") <path>` | blank line then the diff (§5.5) | same |
| `bash` (tool) | `bold("$ <command>")` in `toolTitle`, plus `muted(" (timeout Ns)")` | blank line then the **last 5** visual lines of output in `toolOutput`, preceded by `muted("... (N earlier lines,") + hint + muted(")")` | all output |
| `bash` (tool), always | — | `\n` + `muted("Elapsed 1.2s")` while running, `muted("Took 1.2s")` when done | same |
| `ls` | `bold("ls") <path or "."> ` + `toolOutput(" (limit N)")` | blank line then the first 20 lines | all |
| `grep` / `find` | `bold(name)` + formatted arguments | first N lines | all |

Truncation and limit notices are appended in `warning`, bracketed, e.g.
`[Truncated: showing 200 of 4210 lines (200 line limit)]`,
`[First line exceeds 100KB limit]`, `[Truncated: 12 entries limit, 100KB limit]`.

The expand hint is always spelled the same way: `muted("... (N more lines,")` then a space,
then `keyHint("app.tools.expand", "to expand")` (which renders `dim("ctrl+o") + muted(" to expand")`),
then `muted(")")`.

`ctrl+o` toggles expansion **globally** — every tool block, compaction summary, branch summary,
skill block and custom entry in the transcript flips at once. There is no per-result selection
in ohm. micro's `ctrl+↑/↓` focus model has no ohm counterpart; §9 treats keeping it as a
conscious deviation.

### 5.4 Bash execution (the user's `!` prompt)

`components/bash-execution.ts`. This is the block created when the prompt starts with `!`
(or `!!`, which excludes the output from the model's context).

```
Spacer(1)
─────────────────────────────────────────  DynamicBorder in `bashMode` (or `dim` for `!!`)
 $ <command>                               bold, `bashMode`, paddingX 1
 <output, last 20 visual lines, `muted`>
 <spinner line>  or  <status lines>
─────────────────────────────────────────  DynamicBorder, same colour
```

- Preview limit is **20** logical lines while collapsed; `ctrl+o` shows everything.
- Output is ANSI-stripped and `\r\n`/`\r` normalised as it streams.
- While running: a `Loader` whose message is `Running... (escape to cancel)`, spinner in the
  border colour, message in `muted`.
- On completion the loader is replaced by whichever of these apply, each on its own line after
  a blank line:
  - `muted("... N more lines (") + dim("ctrl+o") + muted(" to expand") + muted(")")`,
    or the collapse form when already expanded.
  - `warning("(cancelled)")`.
  - `error("(exit <code>)")` for a non-zero exit.
  - `warning("Output truncated. Full output: <path>")`.
- Known ohm inconsistency: the header is built with the `dim` colour for `!!` in the
  constructor but rebuilt with `bashMode` in `updateDisplay`, so after the first output chunk a
  `!!` command's header turns green while its rules stay dim. Reproducing this is not required.

### 5.5 Diff

`components/diff.ts` renders; `core/tools/edit-diff.ts:380 generateDiffString` produces the
text.

Line format, where `W` is the width of the largest line number in the file:

```
+<newLineNo padStart W> <content>
-<oldLineNo padStart W> <content>
 <lineNo padStart W> <content>
 <W spaces> ...
```

- Context: **4 lines** each side of a change (`contextLines = 4`). A gap of `≤ 8` context lines
  between two changes is printed whole; a longer one prints 4, then the ` … ...` elision row,
  then 4.
- Tabs inside content become three spaces.
- Colours: `-` lines in `toolDiffRemoved`, `+` lines in `toolDiffAdded`, context in
  `toolDiffContext`. The whole line including the marker and number is coloured.
- **Intra-line highlighting**: when a change is exactly one removed line followed by exactly
  one added line, the two contents are word-diffed and the differing words are wrapped in
  `chalk.inverse` (reverse video) on top of the line colour. Leading whitespace on the first
  changed part is stripped out of the highlight so indentation is never inverted. Runs of more
  than one line on either side are printed plainly, all removals then all additions.
- Anything that does not parse as a diff line is printed in `toolDiffContext` verbatim.

micro's `diff.rs` / `Row::*` produce `+`/`-`/` ` with no line numbers, no elision widths, and
no intra-line inverse. `Row::Divider` renders `⋯`; ohm renders `...` in the number column.

### 5.6 Thinking block

Covered in §5.2. There is no glyph. It is italic body text in `thinkingText`, rendered as
markdown, at `paddingX = 1`, with no background. Toggle is `ctrl+t` (`app.thinking.toggle`).

### 5.7 Compaction summary

`components/compaction-summary-message.ts`. `Spacer(1)`, then a `Box(1, 1, bg = customMessageBg)`:

```
[compaction]                       customMessageLabel, bold (via ESC[1m … ESC[22m)
<blank>
Compacted from 12,345 tokens (ctrl+o to expand)
```

`"Compacted from N tokens ("` in `customMessageText`, `"ctrl+o"` in `dim`, `" to expand)"` in
`customMessageText`. Token count is locale-formatted with thousands separators.

Expanded, the third line becomes markdown of `**Compacted from N tokens**\n\n` + the summary,
coloured `customMessageText`.

### 5.8 Branch summary

`components/branch-summary-message.ts`. Identical shape:

```
[branch]
<blank>
Branch summary (ctrl+o to expand)
```

Expanded: markdown of `**Branch Summary**\n\n` + the summary.

### 5.9 Skill invocation

`components/skill-invocation-message.ts`. A `Box(1, 1, bg = customMessageBg)` with **no**
leading `Spacer` of its own (the caller adds one).

Collapsed — one line:

```
[skill] <name> (ctrl+o to expand)
```

`customMessageLabel(bold("[skill] "))`, then `customMessageText(name)`, then
`dim(" (ctrl+o to expand)")`.

Expanded:

```
[skill]
**<name>**

<full skill content as markdown, in customMessageText>
```

If the user's message contained both a skill block and prose, the prose is rendered as a
separate `UserMessageComponent` after a `Spacer(1)`.

### 5.10 Custom message and custom entry

`components/custom-message.ts` — extension-provided messages. If the extension supplies a
renderer, its component is used verbatim (it owns its own styling). Otherwise:

```
Spacer(1)
Box(1, 1, bg = customMessageBg):
  [<customType>]                    customMessageLabel, bold
  <blank>
  <content as markdown, in customMessageText>
```

`components/custom-entry.ts` — extension-provided session entries. A `Spacer(1)` then whatever
the renderer returns. If the renderer throws, a `customMessageBg` box shows
`error("[<type>] renderer failed: <message>")`.

### 5.11 Status and notice lines

There is no dedicated component. `interactive-mode.ts:3102 showStatus` appends
`Spacer(1)` + `Text(dim(message), 1, 0)`. Consecutive status updates **mutate the same
`Text`** rather than appending, so a stream of status changes does not spam the transcript.

Other one-off lines follow the same shape at `paddingX = 1`:

- errors: `Text(error("Error: <msg>"), 1, 0)`
- warnings: `Text(warning("Warning: <msg>"), 1, 0)`
- cache-miss notice: `warning("Cache miss: 42k tokens re-billed (~$0.31)")`, with the label
  becoming `"Cache miss after model switch"` or `"Cache miss after 12m idle"`. Suppressed
  below 20,000 tokens and $0.10.
- untrusted project: `warning("This project is not trusted. Project .ohm resources and packages are ignored. Use /trust to save a trust decision, then restart ohm.")`

No marker glyph is used for any of these. micro's `· `, `⟳ `, `✗ ` prefixes are additions.

---

## 6. The editor

Sources: `packages/tui/src/components/editor.ts`, `editor-component.ts`,
`word-navigation.ts`, `kill-ring.ts`, `undo-stack.ts`, `keybindings.ts`, and
`packages/coding-agent/src/modes/interactive/components/custom-editor.ts`.

### 6.1 Keybinding table

Every binding is a named action resolved through `KeybindingsManager`, so users can remap any
of them. Defaults:

| Action id | Default keys | Effect |
|---|---|---|
| `tui.editor.cursorUp` | `up` | Up one **visual** row; at the top, browse prompt history back; at the top with no history, jump to line start |
| `tui.editor.cursorDown` | `down` | Down one visual row; at the bottom while browsing history, forward; else jump to line end |
| `tui.editor.cursorLeft` | `left`, `ctrl+b` | Left one grapheme; wraps to the end of the previous logical line |
| `tui.editor.cursorRight` | `right`, `ctrl+f` | Right one grapheme; wraps to the start of the next logical line |
| `tui.editor.cursorWordLeft` | `alt+left`, `ctrl+left`, `alt+b` | Word left (§6.2) |
| `tui.editor.cursorWordRight` | `alt+right`, `ctrl+right`, `alt+f` | Word right |
| `tui.editor.cursorLineStart` | `home`, `ctrl+a` | Column 0 of the logical line |
| `tui.editor.cursorLineEnd` | `end`, `ctrl+e` | End of the logical line |
| `tui.editor.jumpForward` | `ctrl+]` | Arm jump-to-char forward (§6.6) |
| `tui.editor.jumpBackward` | `ctrl+alt+]` | Arm jump-to-char backward |
| `tui.editor.pageUp` | `pageUp` | Up `max(5, floor(rows*0.3))` visual rows |
| `tui.editor.pageDown` | `pageDown` | Down the same |
| `tui.editor.deleteCharBackward` | `backspace` (also `shift+backspace`) | Delete one grapheme back; at column 0 merge with the previous line |
| `tui.editor.deleteCharForward` | `delete`, `ctrl+d` (also `shift+delete`) | Delete one grapheme forward; at end of line merge the next line |
| `tui.editor.deleteWordBackward` | `ctrl+w`, `alt+backspace` | Kill to the previous word boundary |
| `tui.editor.deleteWordForward` | `alt+d`, `alt+delete` | Kill to the next word boundary |
| `tui.editor.deleteToLineStart` | `ctrl+u` | Kill from line start to cursor |
| `tui.editor.deleteToLineEnd` | `ctrl+k` | Kill from cursor to line end; at end of line, kill the newline |
| `tui.editor.yank` | `ctrl+y` | Insert the most recent kill |
| `tui.editor.yankPop` | `alt+y` | Cycle to the previous kill (only immediately after a yank) |
| `tui.editor.undo` | `ctrl+-` | Pop one undo snapshot |
| `tui.input.newLine` | `shift+enter`, `ctrl+j` | Insert a line break |
| `tui.input.submit` | `enter` | Submit |
| `tui.input.tab` | `tab` | Completion (§2) |
| `tui.input.copy` | `ctrl+c` | Ignored by the editor; the app handles it |
| `tui.select.up` / `down` | `up` / `down` | Menu navigation |
| `tui.select.pageUp` / `pageDown` | `pageUp` / `pageDown` | Menu paging |
| `tui.select.confirm` | `enter` | Menu confirm |
| `tui.select.cancel` | `escape`, `ctrl+c` | Menu cancel |

App-level bindings, intercepted by `CustomEditor` **before** the editor sees the key:

| Action id | Default keys | Effect |
|---|---|---|
| `app.interrupt` | `escape` | Abort the turn. Only when no autocomplete menu is open — otherwise Escape closes the menu |
| `app.clear` | `ctrl+c` | Clear the editor; pressed twice in quick succession, exit |
| `app.exit` | `ctrl+d` | Exit, but only when the editor is empty; otherwise it falls through to delete-forward |
| `app.suspend` | `ctrl+z` (not on Windows) | Suspend to background |
| `app.thinking.cycle` | `shift+tab` | Cycle the thinking level (recolours the editor rules) |
| `app.model.cycleForward` | `ctrl+p` | Next model |
| `app.model.cycleBackward` | `shift+ctrl+p` | Previous model |
| `app.model.select` | `ctrl+l` | Open the model selector |
| `app.tools.expand` | `ctrl+o` | Toggle **all** collapsible output |
| `app.thinking.toggle` | `ctrl+t` | Show/hide thinking blocks |
| `app.editor.external` | `ctrl+g` | Open `$EDITOR` on the prompt |
| `app.message.copy` | `ctrl+x` | Copy a message to the clipboard |
| `app.message.followUp` | `alt+enter` | Queue the prompt as a follow-up |
| `app.message.dequeue` | `alt+up` | Pull all queued messages back into the editor |
| `app.clipboard.pasteImage` | `ctrl+v` (`alt+v` on Windows) | Paste an image, falling back to text |

Also unbound-by-default but present: `app.session.new`, `app.session.tree`,
`app.session.fork`, `app.session.resume`. Selector-local bindings (`app.tree.*`,
`app.session.*`, `app.models.*`) only apply inside their dialog.

`shift+space` inserts a literal space (some terminals send it as a distinct sequence).

### 6.2 Word motion

`word-navigation.ts`, driven by `Intl.Segmenter` word segmentation, not a character class.

Backward from the cursor:

1. Pop trailing whitespace segments.
2. If the last remaining segment is an **atomic** segment (a paste marker), skip exactly it.
3. If it is word-like, skip inside it, but stop at the last ASCII punctuation boundary within
   the segment — `PUNCTUATION_REGEX = /[(){}[\]<>.,;:'"!?+\-=*/\\|&%^$#@~`]/`. So
   `foo.bar|` moves to `foo.|bar`, not to `|foo.bar`.
4. Otherwise skip the whole run of non-word, non-whitespace segments.

Forward is the mirror image: skip leading whitespace, then one atomic segment, or up to the
first punctuation inside the word-like segment, or the whole punctuation run.

At column 0, word-left moves to the **end of the previous logical line**. At end of line,
word-right moves to **column 0 of the next line**.

micro's `word_start_before`/`word_end_after` use `is_alphanumeric() || '_'`, which differs:
ohm treats `foo.bar` as three stops, micro as two.

### 6.3 Kill ring

`kill-ring.ts`. A stack; the most recent kill is the last element.

- All five kill operations push onto it: `ctrl+u`, `ctrl+k`, `ctrl+w`/`alt+backspace`,
  `alt+d`/`alt+delete`.
- `accumulate` is true when the previous action was also a kill, so consecutive kills merge
  into one entry. Backward kills **prepend** to the entry, forward kills **append**, which
  keeps the merged text in reading order.
- Killing a newline (at a line boundary) pushes a literal `"\n"` with the same merge rules.
- `ctrl+y` (yank) inserts `peek()` and sets `lastAction = "yank"`.
- `alt+y` (yank-pop) works **only** when `lastAction === "yank"` and the ring holds more than
  one entry: it deletes the text just yanked, rotates the ring (moves the last entry to the
  front), and inserts the new last entry.
- Plain backspace and delete-forward do **not** touch the ring, and they reset `lastAction`.

### 6.4 Undo granularity

`undo-stack.ts` — a stack of deep-cloned `{lines, cursorLine, cursorCol}` snapshots. Fish-style
coalescing:

- Typing a **word character** pushes a snapshot only when the previous action was not
  `"type-word"`. A run of word characters is therefore one undo unit.
- Typing **whitespace** always pushes a snapshot first, so a space is separately undoable and
  undoing removes the space plus the word that followed it.
- Every deletion, newline, paste, yank, autocomplete commit, history navigation entry, and
  programmatic `setText` pushes exactly one snapshot. Each is atomic.
- Submitting clears the stack.
- `ctrl+-` pops one snapshot, restores it, resets `lastAction` and the sticky column.

### 6.5 Submit rules

- `enter` submits, unless a menu is open (§2.6).
- **Backslash escape**: if the character immediately before the cursor is `\`, Enter deletes
  the backslash and inserts a newline instead of submitting. This is the workaround for
  terminals with no `shift+enter`.
- The reverse case: if `shift+enter` has been remapped onto submit, then pressing plain Enter
  after a `\` deletes the backslash and *submits*.
- `shift+enter` and `ctrl+j` always insert a newline. So do several raw sequences ohm sniffs
  for directly: `\x1b\r`, `\x1b[13;2~`, a lone `\n`, and any multi-byte data containing both
  ESC and CR.
- On submit: paste markers are expanded to their real content, the result is `trim()`ed, the
  buffer is emptied, the paste map cleared, history browsing exited, scroll reset, and the undo
  stack cleared. `onChange("")` fires before `onSubmit(text)`.

### 6.6 Paste handling

- Bracketed paste (`ESC[200~` … `ESC[201~`) is buffered until the terminator, then processed as
  one unit. Anything after the terminator is re-fed through `handleInput`.
- Some terminals re-encode control bytes inside a paste as CSI-u `ESC[<code>;5u`; these are
  decoded back to the raw control byte before filtering, so newlines survive.
- Cleaning: `\r\n` and `\r` → `\n`, tabs → **four spaces**, then every character below `0x20`
  except `\n` is dropped.
- If the paste starts with `/`, `~`, or `.` and the character before the cursor is a word
  character, a space is inserted first.
- **Large paste** — more than **10 lines** or more than **1000 characters** — is not inserted.
  It is stored under an id and a marker is inserted instead:
  `[paste #1 +123 lines]` or `[paste #1 4210 chars]` (the line form wins when both apply).
  The marker is an *atomic* segment: cursor motion, word motion, backspace, and word wrap all
  treat it as one unit. Backspacing over a marker deletes the stored paste and renumbers every
  higher marker down by one.
- `getExpandedText()` substitutes markers back to content; the editor's own `getText()` does
  not. Submit uses the expanded form.
- A paste is one undo unit and never triggers autocomplete.

### 6.7 Wrapping and vertical motion

`wordWrapLine` (`editor.ts:114`) wraps at the last whitespace before a non-whitespace grapheme,
falls back to a hard break when no wrap opportunity fits, and allows a break on either side of
a CJK character (`Han`, `Hiragana`, `Katakana`, `Hangul`, `Bopomofo`). Wide characters count
2 columns. An atomic segment wider than the line is broken visually but stays atomic for
editing.

Vertical motion works on visual rows and keeps a **sticky column**. `computeVerticalMoveColumn`
implements this table, where P = a preferred column is remembered, S = the cursor is not at the
end of its row, T = the target row is shorter than the current visual column, U = the target row
is shorter than the preferred column:

| P | S | T | U | Result |
|---|---|---|---|---|
| 0 | * | 0 | – | clear preferred; go to the same visual column |
| 0 | * | 1 | – | remember the current column; go to the target's end |
| 1 | 0 | 0 | 0 | clear preferred; go to the preferred column |
| 1 | 0 | 0 | 1 | keep preferred; go to the target's end |
| 1 | 0 | 1 | – | keep preferred; go to the target's end |
| 1 | 1 | 0 | – | clear preferred; go to the same visual column |
| 1 | 1 | 1 | – | remember the current column; go to the target's end |

Any horizontal motion or edit clears the sticky column. When a vertical move would land inside
an atomic segment, the cursor snaps to the segment's start and the pre-snap column is
remembered so the next vertical move resolves correctly.

### 6.8 Prompt history

- `addToHistory(text)` is called after each successful submit and when restoring a session.
  Text is trimmed; empty is ignored; a duplicate of the most recent entry is ignored; the list
  is capped at **100** entries, newest first.
- `up` browses back only when the cursor is on the first visual row **and** (the editor is
  empty, or history browsing is already active, or the cursor is at column 0).
- `down` browses forward only when browsing is active and the cursor is on the last visual row.
- Entering history browsing snapshots the in-progress draft; returning past the newest entry
  restores it.
- Browsing back places the cursor at the **start**; browsing forward places it at the **end**.

### 6.9 Jump to character

`ctrl+]` then any printable character moves the cursor to the next occurrence of that
character, searching the current line from `cursor+1` and then every following line.
`ctrl+alt+]` searches backward the same way. Case-sensitive. Pressing the trigger again, or any
control character, cancels. No match leaves the cursor alone.

---

## 7. Selectors and dialogs

### 7.1 The shared pattern

Every dialog replaces the editor inside `editorContainer` — it is not an overlay, and it is not
centred. It occupies the same slot in the vertical flow, so the transcript above it is
untouched and the footer stays below.

The common skeleton:

```
DynamicBorder()                       full-width ─ rule in `border` (#5f87ff)
[Spacer(1)]
[Text(bold(accent(<title>)), 1, 0)]
[Text(muted(<subtitle / hint>), 1, 0)]
[Spacer(1)]
[Input]                               a "> " prompt line with a reverse-video cursor
<list container>                      SelectList, SettingsList, or hand-rolled rows
[Spacer(1)]
[Text(<keybinding hints>, 1, 0)]
[Spacer(1)]
DynamicBorder()
```

Row convention everywhere: `"→ "` prefix in `accent` for the selected row, two spaces
otherwise; the selected row's text is `accent`; a `success("✓")` suffix marks the current
value; scroll position renders as `  (i/n)` in `muted`.

Navigation convention: `up`/`down` wrap at both ends, `enter` confirms, `escape` or `ctrl+c`
cancels. Where a search input is present, every unhandled key is forwarded to it and the list
is re-filtered on each keystroke.

`Input` (`packages/tui/src/components/input.ts`) is one line: the literal prompt `"> "`, the
value with horizontal scrolling that keeps the cursor near the middle once the text overflows,
a reverse-video cursor (`ESC[7m…ESC[27m`), and space padding to the full width. It has its own
kill ring and undo stack and supports the same editing keys as the editor minus the multi-line
ones.

### 7.2 Per-dialog specifics

**Model selector** (`model-selector.ts`, `ctrl+l` or `/model`) — border, blank, then either
`Scope: all | scoped` with the active side in `accent` and a `tab scope (all/scoped)` hint, or
`warning("Only showing models from configured providers. Use /login to add providers.")`.
Then the search `Input`, then
`muted("Filter by provider: api: input: reasoning:  e.g. provider:google input:image")`.
The list shows 10 rows: `→ <id> [<provider>] ✓`, id in `accent` when selected, provider badge
always `muted`, checkmark in `success` for the current model. Below the list, a blank line and
`muted("  Model Name: <display name>")`. Filtering is `key:value` prefixes parsed out first,
then `fuzzyFilter` over `id provider name`. Refresh status appears as
`success("  Model catalogs refreshed.")` or an `error` line.

**Session selector** (`session-selector.ts`, `/resume`) — `Spacer`, a rule in **`accent`**
rather than `border`, `Spacer`, a header block (scope, sort mode, name filter, load progress,
delete confirmation, status messages), the list, `Spacer`, and a second `accent` rule. Search
is fuzzy over session names and paths; `ctrl+p` toggles path display, `ctrl+s` sort mode,
`ctrl+n` the named-only filter, `ctrl+r` rename, `ctrl+d` delete with confirmation.
Relative dates are formatted `now`, `5m`, `3h`, `2d`, `3w`, `4mo`, `2y`.

**Theme selector** (`theme-selector.ts`, `/theme`) — border, a 10-row `SelectList` over
available theme names with `(current)` as the description of the active one, border. Moving
the selection **live-previews** the theme; Escape restores.

**Settings selector** (`settings-selector.ts`, `/settings`) — border, a `SettingsList` with
`maxVisible = 10` and search enabled, border. Each row is `→ ` + the label padded to
`min(30, longest label)` + two spaces + the current value. Label and value are `accent` when
selected, otherwise the label is default-coloured and the value is `muted`. The selected row's
description is shown below the list after a blank line, wrapped to `width - 4`, indented two
spaces, in `dim`. `enter` or `space` cycles the value or opens a submenu. Submenus replace the
list entirely and are titled `bold(accent(title))` with `dim("  Enter to select · Esc to go back")`
at the bottom.

**Thinking selector** (`thinking-selector.ts`, `/thinking`) — border, a `SelectList` sized to
the number of levels, border. Descriptions:
`off` → `No reasoning`; `minimal` → `Very brief reasoning (~1k tokens)`;
`low` → `Light reasoning (~2k tokens)`; `medium` → `Moderate reasoning (~8k tokens)`;
`high` → `Deep reasoning (~16k tokens)`; `xhigh` → `Extra-high reasoning (~32k tokens)`;
`max` → `Maximum reasoning`.

**Trust selector** (`trust-selector.ts`, `/trust`) — border, blank,
`bold(accent("Project trust"))`, `muted(<cwd>)`, blank,
`muted("Saved decision: <trusted|untrusted> (<path>)")` or `none` or
`… (inherited from <path>)`, `muted("Current session: trusted|untrusted")`, blank, the option
rows, blank, the hint line `↑↓ navigate  enter save  escape/ctrl+c cancel`, blank, border.
Also accepts `j`/`k` for movement.

**Login dialog** (`login-dialog.ts`, `/login`) — border, `bold(accent("Login to <provider>"))`,
a content area, border. The content area is filled step by step by the auth flow: the URL as an
OSC 8 hyperlink in `accent`, then `dim("Cmd+click to open")` (`Ctrl+click` off macOS), any
`warning` instructions, a device code as `warning("Enter code: ABCD-1234")`, and prompts with an
`Input` plus `(escape to cancel, enter to submit)`. Submitted input is replaced in place with
`> <value>`. The browser is opened automatically.

**Tree selector** (`tree-selector.ts`, `/tree`) — `Spacer`, border, `bold("  Session Tree")`,
border, `Spacer`, the tree, `Spacer`, border. Filter modes bound to `ctrl+d` (default),
`ctrl+t` (no tools), `ctrl+u` (user only), `ctrl+l` (labeled only), `ctrl+a` (all), with
`ctrl+o`/`ctrl+shift+o` cycling. `alt/ctrl+left` folds or moves up, `alt/ctrl+right` unfolds or
moves down, `shift+l` edits a label, `shift+t` toggles label timestamps.

**Bordered loader** (`bordered-loader.ts`) — the pattern for a modal wait:
border, the spinner line, blank, `dim("escape") + muted(" cancel")`, blank, border.

**Extension selector / input / editor** (`extension-selector.ts`, `extension-input.ts`,
`extension-editor.ts`) — the same skeleton, driven by an extension's schema.

micro has none of these. Its `render/approval.rs` is the only dialog and is a micro-only
concept; the cheapest parity move is to restyle it into the skeleton above — border rules in
`border`, a `bold(accent(...))` title, `→ ` option rows in `accent`, hints in
`dim`+`muted` — rather than the current `surface`-tinted band.

---

## 8. Status, loader, and footer

### 8.1 The status area

`statusContainer` always renders **two rows**, whether idle or working. `IdleStatus` emits two
width-filled blank lines; a running `Loader` emits one blank line followed by one text line
(`Loader.render` prepends `""` to `Text.render`). The interface therefore never shifts
vertically when a turn starts.

### 8.2 Spinner

`packages/tui/src/components/loader.ts`:

- Frames: `⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏` — ten braille frames.
- Interval: **80 ms** (`DEFAULT_INTERVAL_MS`). A single-frame or empty frame list disables the
  timer entirely.
- Rendered as `<spinnerColorFn(frame)> <messageColorFn(message)>` at `paddingX = 1`,
  `paddingY = 0`. When the frame is empty the leading space is omitted too.
- Extensions can override frames and interval; when they do, the frame is emitted verbatim
  without the colour function.

micro's `SPINNER` in `render/status.rs` is the same ten frames, advanced once per render tick
rather than on an 80 ms timer.

### 8.3 The four status indicators

`components/status-indicator.ts`:

| Kind | Spinner colour | Message |
|---|---|---|
| working | `accent` | `Working...` in `muted`, or an extension-supplied message |
| retry | `warning` | `Retrying (2/5) in 8s... (escape to cancel)` in `muted`, counting down once a second |
| compaction | `accent` | `Compacting context... (escape to cancel)`, or `Auto-compacting... (escape to cancel)`, or `Context overflow detected, Auto-compacting... (escape to cancel)` |
| branch summary | `accent` | `Summarizing branch... (escape to cancel)` |

The countdown is driven by `components/countdown-timer.ts`: a 1 Hz interval that recomputes the
message and requests a render, disposing itself at zero.

ohm shows **no elapsed timer** in the status line. Elapsed time appears only inside a bash tool
block, as `muted("Elapsed 3.4s")` while running and `muted("Took 3.4s")` when finished.
micro's `working  12s  ctrl+c to interrupt` merges the two ideas; ohm's equivalent is
`Working...` alone, with the interrupt hint living in the startup header.

### 8.4 Keybinding hints

`components/keybinding-hints.ts` is the single source of hint formatting:

```
keyHint(action, description)  ->  dim(<keys>) + muted(" " + description)
rawKeyHint(keys, description) ->  dim(<formatted keys>) + muted(" " + description)
```

Multiple keys for one action join with `/` (`ctrl+c/ctrl+d`). Multi-part keys join with `+`.
On macOS the part `alt` displays as `option`. `keyDisplayText` capitalises each part
(`Ctrl+O`), plain `keyText` does not (`ctrl+o`) — the transcript uses the lowercase form.

### 8.5 The footer

`components/footer.ts`. Two lines, sometimes three. **No background fill.** Both lines are
`dim` overall.

Line 1 — location:

```
~/code/biz/micro (main) • my-session-name
```

- cwd with `$HOME` replaced by `~` (only when the path is genuinely inside home).
- ` (branch)` appended when a git branch is known; the branch is watched and updates live.
- ` • <session name>` appended when the session has one.
- Truncated to the terminal width with a `dim("...")` ellipsis.

Line 2 — usage on the left, model right-aligned:

```
↑1.2k ↓340 R45k W2.1k CH92.4% $0.031 12.4%/200k (auto)          (anthropic) claude-opus-5 • high
```

Parts, in order, each omitted when zero:

| Part | Format |
|---|---|
| input tokens | `↑` + `formatTokens` |
| output tokens | `↓` + `formatTokens` |
| cache reads | `R` + `formatTokens` |
| cache writes | `W` + `formatTokens` |
| cache hit rate | `CH` + one decimal + `%`, only when there was any cache activity |
| cost | `$` + three decimals, plus ` (sub)` when the provider is on an OAuth subscription |
| context | `<pct with one decimal>%/<window>` or `?/<window>` right after a compaction, plus ` (auto)` when auto-compaction is on |
| experimental flag | `dim("•") + bold(warning("xp"))` when experimental features are enabled |

The context part is the only coloured element: `error` above 90%, `warning` above 70%,
otherwise inherited `dim`.

The right side is `<model id>`, or `<model id> • <thinking level>` / `<model id> • thinking off`
when the model supports reasoning, optionally prefixed with `(<provider>) ` when more than one
provider is configured and it fits. A minimum of 2 spaces separates the halves; the left side
truncates with `...` first, then the right side truncates with no ellipsis.

`formatTokens`: `<1000` → as-is; `<10000` → one decimal + `k`; `<1e6` → rounded + `k`;
`<1e7` → one decimal + `M`; else rounded + `M`. micro's `format_tokens` already matches.

Line 3, only when extensions publish statuses: their values sorted by key, joined with a
space, control characters collapsed to single spaces, truncated with a `dim("...")` ellipsis.

Notable: ohm's totals are **cumulative over the whole session** (summed across every assistant
message), while micro reports `last_usage`. And ohm derives the context percentage from the
session's own accounting, which survives compaction; micro recomputes it from the last turn.

---

## 9. Prioritised implementation checklist

Ordered by visual return per unit of work. Items 1–8 get most of the way to "this looks like
ohm"; 9 onward close the remaining gaps.

**Tier 1 — the look, cheap**

1. **Replace `theme.rs` with ohm's 50 tokens.** Transcribe §3.1 verbatim, including the light
   palette, and route every existing `theme.dim` / `theme.muted` / `theme.warning` use through
   the corrected values. Everything below depends on these names existing. Add JSON loading and
   the `vars` indirection afterwards; the hardcoded structs are enough to start.
2. **User messages get the `userMessageBg` band.** Drop the `› ` prefix. `Box`-equivalent:
   one blank tinted row, body at 1-column inset, one blank tinted row, tint to full width.
   Single biggest recognisability win.
3. **Tool blocks get the pending/success/error band.** Same box shape, background switching on
   state. Drop the `⏺ ` marker; the header becomes `bold(toolTitle, name)` + `accent(path)`.
4. **The editor gets rules instead of a tint.** A `─`-filled line above and below in
   `borderMuted`, and colour them by thinking level / bash mode once those exist. Remove the
   `surface` band and the placeholder text — ohm shows an empty line with a cursor.
5. **Footer: two lines, no background.** Line 1 cwd + branch + session; line 2 cumulative
   usage + right-aligned model + thinking level. Drop the `status` tint.

**Tier 2 — behaviour that is felt immediately**

6. **Autocomplete parity (§2).** Pre-select the best match; make Enter on a slash command
   commit *and* submit; render the menu with the `→ `/`  ` prefix, the 12–32 column name
   column, `accent` for the whole selected row, `muted` descriptions, and the `  (i/n)` scroll
   row. `fuzzy.rs` is already correct.
7. **Diff with line numbers and intra-line highlighting (§5.5).** `+NNN` / `-NNN` / ` NNN`,
   4 lines of context, `...` elision, word-level reverse video on single-line changes.
8. **`ctrl+o` becomes global.** Every collapsible thing expands at once. Decide explicitly
   whether to keep micro's `ctrl+↑/↓` focus model as an addition — it has no ohm counterpart,
   and the `ctrl+o` affordance text (`… +N lines  ctrl+o`) currently appears only on the
   focused result, which ohm never does.

**Tier 3 — the structural change**

9. **Move to a scrollback-native render model (§1.1).** ratatui `Viewport::Inline` plus
   `Terminal::insert_before`: completed transcript entries are pushed into the terminal's
   scrollback and never redrawn; the live region is the streaming message, the status rows, the
   editor, and the footer. Delete `scroll.rs` and the mouse-wheel handling once this lands.
   This is a day of work and it changes every other file, so it should come after Tier 1 has
   settled the visual vocabulary.

**Tier 4 — editor depth (§6)**

10. Kill ring with accumulation and `ctrl+y` / `alt+y`.
11. Undo stack with fish-style coalescing on `ctrl+-`.
12. Prompt history on `up`/`down` at the buffer edges, 100 entries, draft preservation.
13. Large-paste markers (`[paste #1 +123 lines]`) with atomic-segment semantics.
14. `Intl.Segmenter`-equivalent word motion with the punctuation-boundary rule
    (`unicode-segmentation`'s `unicode_words` plus ohm's `PUNCTUATION_REGEX` set).
15. Sticky-column vertical motion per the table in §6.7. micro has a simpler sticky column
    already; the table adds the "target too short" cases.
16. Backslash-Enter newline escape, jump-to-char (`ctrl+]`), page motion at 30% of the
    terminal height, and the `max(5, rows*0.3)` editor height cap (currently a flat 10).

**Tier 5 — missing surfaces**

17. Startup header, collapsible on `ctrl+o`, with the exact hint list in §4.4.
18. Status indicators: `Working...` in `muted` with an `accent` spinner on an 80 ms timer,
    always two rows, plus the retry/compaction/branch-summary variants.
19. Bash execution blocks for `!` and `!!` prompts (§5.4).
20. Markdown gaps: links (with OSC 8), tables, task lists, nested and ordered lists, `*`/`_`
    emphasis, strikethrough, printed code fences, `─`×80 rules, syntax highlighting.
21. Selectors: model, theme, settings, thinking, session, trust, login — each following the
    §7.1 skeleton. Restyle the approval prompt to match while you are there.
22. Compaction, branch-summary, and skill blocks (§5.7–5.9), if micro grows those features.
23. OSC 133 prompt zones on user and assistant messages — four escape sequences, and terminal
     prompt-jumping starts working.

### What cannot be reproduced as-is in ratatui

- **Nothing here is impossible**, but two things are awkward:
  - **The append-and-diff render model.** ratatui's `Terminal` owns a buffer and repaints it.
    `Viewport::Inline` + `insert_before` gets the same user-visible result — history in the
    terminal's scrollback, a live region at the bottom — but the mechanism is different, so a
    line-for-line port of `tui.ts` is not the right move. Rebuild the behaviour, not the code.
  - **`chalk.inverse` nesting inside a coloured line.** ratatui expresses this as
    `Modifier::REVERSED` on a `Span`, which is equivalent; the only cost is that ohm's diff
    renderer returns a single ANSI-laden string, so micro needs the intra-line diff to produce
    spans instead. Straightforward, just not a copy.
- **Inline images** (kitty/iterm graphics protocols) sit outside ratatui's cell model entirely.
  They can be emitted by writing escape sequences past the buffer and reserving rows, which is
  what ohm does, but it fights the framework. Lowest priority; listed last for that reason.
- **Live theme file watching** and **user-supplied JSON themes** are pure plumbing and carry no
  ratatui difficulty — they are only listed late because nothing visual depends on them until
  §3 lands.

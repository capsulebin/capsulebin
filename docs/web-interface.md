# Web interface

The page design of the browser client (`src/server/web/`): what the create, sealed, view, manage
and state pages show, in which hierarchy, and the shared components behind them. The capsule
content is the focus of every page: header, footer and controls are quiet, the accent colour is
kept for the active tab and the one primary action, and every state page (progress, password,
warning, gone) carries one symbol, the state hexagon. The design uses only the colour tokens of
`tokens.css`, so it follows the operator's `[branding.colors]` ([branding.md](branding.md)), the
system preference and the theme toggle (ADR-005); it changes nothing in the CSP, the server, the
capsule format or the crypto. Languages and the language switch's behaviour are described in
[i18n.md](i18n.md), the creation token's create-page phases in `AGENTS.md` (ADR-018).

## Rules

### Shared chrome (every page)

- **Header and footer** sit on the page background: no band, no border, no accent fill. The
  header holds the logo/wordmark regions and the page claim from [branding.md](branding.md), then
  the language switch and the theme toggle (`.cb-head-tools`). Below 34rem the claim is hidden.
- **Language switch** (`nav.cb-lang[data-language-switch]`, filled by `mountLanguageSwitch` in
  `pages/shell.ts`): one quiet text button (`.cb-quiet`) per offered language, named in that
  language; the page language has `aria-current="true"` and reads in the text colour at weight 600,
  the others muted. No fill. Behaviour (navigate or store-only) is unchanged, see
  [i18n.md](i18n.md#language-switch).
- **Theme toggle** (`[data-theme-toggle]`): an inline SVG icon, never a text glyph. The shells
  carry the "auto" icon (`theme`: a half-filled circle) so the first paint is right; `theme.ts`
  swaps it for `sun` (light) or `moon` (dark). The accessible name is always `theme.switch`; the
  current choice is the button's `title` (`theme.auto`/`theme.light`/`theme.dark`). No shell or
  source contains the `◐` glyph.
- **Radius scale by role** (`tokens.css`): `--cb-radius-control` (8 px: buttons, inputs, selects)
  and `--cb-radius-surface` (12 px: view sheet, editor frame, link panel, delete-link row, menus).
  Not colour tokens, not configurable.
- **Motion:** buttons, tabs and menu buttons change `background-color` and `color` over
  `--cb-duration-quick` (120 ms). Under `prefers-reduced-motion: reduce` a global rule in
  `base.css` sets every transition and animation duration and delay to 0 s (an animation still
  ends, so `animationend` fires and the entry animation's class is removed); the hexagon's own
  rules additionally switch its animations off.
- **Bar actions** (`.cb-quiet` inside `.cb-bar-actions`, the previous/next row, the language
  switch): borderless icon + label buttons. Below 34rem the label (`.cb-bar-label`) is visually
  hidden and only the icon shows; the label stays the accessible name.
- **"Copied" state** on every copy button (view "Copy", sealed "Copy link", delete-link "Copy"),
  through `CopyFeedback` (`pages/copy-state.ts`): after the clipboard accepted the text, the button
  shows a check icon and `shell.copied` with the class `.cb-copied` (accent text on
  `tint_accent`) for `COPIED_FOR_MS` (2000 ms), and a polite live region says
  `shell.copiedToClipboard`. Then the button gets back its own child nodes (parked in a hidden
  element meanwhile, so Lit keeps owning them) and its `aria-label`. A label that narrow screens
  hide stays hidden in the copied state too. One `CopyFeedback` per page: a second copy restores
  the previous button first; the view page restores it also when another item is selected. A
  refused clipboard leaves the button unchanged.
- **No inline `style`** on any element, also not through the CSSOM: everything is classes in the
  bundled stylesheets, and the size bar's fill is an SVG `width` presentation attribute.

### State hexagon

`hexagon(state)` (`pages/hexagon.ts`) returns a decorative `<svg class="cb-hex" data-hexagon="…">`
(`aria-hidden`), built as DOM so plain-DOM pages append it and Lit templates embed it. It is
CapsuleBin's own symbol and appears also on an instance with an operator logo: it signals a state,
not the brand.

| State | Look | Used for |
| --- | --- | --- |
| `running` | a dash runs around the outline (1.1 s, infinite); static full outline with reduced motion | view opening and decrypting, the key derivation inside the password form, create encrypting, manage loading and deleting |
| `sealed` | the outline closes once (0.7 s); drawn closed at once with reduced motion; lock inside | the sealed page |
| `locked` | static, lock inside | view password prompt |
| `warning` | static, stroke `--cb-warning`, exclamation mark inside | view burn warning, manage delete confirmation |
| `empty` | dashed, muted, nothing inside | view already retrieved and unavailable, create error, manage deleted and unavailable |

Colours come from the tokens only: an `accent` outline on a `tint_accent` fill, for `warning` the
`warning` colour on `tint_warning`, for `empty` `text_muted` on `surface_sunken`.

Every progress state is `running`, every irreversible confirmation `warning`, every terminal
"gone/failed" state `empty`. The animations are CSS `stroke-dashoffset` keyframes on the outline
path (`pathLength="60"`); no script animates anything.

### View page (`/p/{id}`), display phase

Top to bottom:

1. **Title** — the capsule title as `h1.cb-h1` (1.5rem), or an `h1.cb-sr-only` with
   `view.capsule` when there is none.
2. **Chip row** (`ul.cb-chips[data-chips]`), in this order: "Deleted after this retrieval"
   (`view.deletedAfterRetrieval`, only for a burn-after-retrieval capsule, `.cb-chip-warning`:
   warning colour); the expiry (`formatExpiry`, clock icon); the size (stack icon), prefixed with
   `view.itemCount` and " · " when there is more than one item; "Decrypted locally"
   (`view.decryptedLocally`, lock icon). The burn flag comes from the descriptor
   (`burnAfterRetrieval` of `openCapsule`'s outcome).
3. **One sheet** (`.cb-sheet.cb-view-sheet[data-sheet]`) holding:
   - the head row (`.cb-sheet-bar`, sticky at the top while a long item scrolls, opaque): with
     several items the tab bar and the "All N" button, with one item its name and size in place
     of the tabs; then the item actions (`.cb-bar-actions`): for Markdown the Preview/Source
     switch, "Copy" (not for files) and "Download";
   - the item panel (`.cb-item-panel`): the content straight on the sheet — no card, border or
     shadow on prose, `pre`, code block or file card;
   - with two or more items, the previous/next row (`.cb-item-nav`).

Details:

- **Preview/Source** (`.cb-view-switch`) is a quiet two-button switch; the pressed half is lifted
  (surface background, text colour, weight 600), never accent. In the display phase the only
  accent-coloured indicator is the active tab; nothing has an accent background.
- **Reading measure:** rendered Markdown is at most `--cb-measure` (72ch) wide, its padding
  outside that width (`box-sizing: content-box`).
- **Line numbers** only for `code` items: a gutter `.cb-code-gutter` (`aria-hidden="true"`,
  `user-select: none`) next to `<pre class="cb-code-block"><code>`, holding 1…N where N is the
  number of lines as `<pre>` shows them (a final newline starts no line; at least 1). `<code>`
  keeps exactly the item text and the unsplit highlight.js output, so a multi-line comment stays
  one span and "Copy" and selection never pick up a number. Gutter and code share font and line
  height so the numbers stay level. Plain text and Markdown source have no gutter.
- **Previous/next row:** a "previous" button with `aria-label` `items.previous` showing the
  previous item's name, the position `items.position` ("2 of 5"), a "next" button (`items.next`)
  with the next item's name. At the ends the button is disabled and `visibility: hidden` (it keeps
  its place). Selecting an item restores a pending "Copied", re-renders, and `revealActiveItem`
  scrolls the item's start below the sticky head row when the page is scrolled past it.
- **Narrow screens** (below 34rem): actions and previous/next show icons only; at 400 px nothing
  overflows the column or the sheet.

### Tab overflow: "All N"

On the view and the create page `fitTabs` (`pages/item-tabs.ts`, unchanged rule) hides the tabs
that do not fit, keeping the active one visible. Only then the "All N" button (`items.all`,
N = number of items, chevron icon) appears. Its menu lists **every** item in manifest (view) or
creation (create) order, whatever the width hides, so an item is always at the same place; the
active row is `aria-checked="true"`.

- View rows (`.cb-menu-row`, `menuitemradio`): kind badge, name, size (`.cb-menu-size`).
- Create rows (`.cb-menu-line` with `.cb-menu-row` and, while there is more than one item, a
  remove control `.cb-menu-remove` named `create.removeItem` "Remove {name}").
- Choosing a row selects the item, closes the menu and focuses its tab; Escape closes the menu
  and returns focus to the button (on the view page after Lit's `updateComplete`); arrow keys move
  within it; a click outside closes it (create: by the event's `composedPath`, because a removed
  row is no longer inside the menu). Removing an item from the create menu keeps the menu open on
  the checked row, or focuses the active tab once every tab fits again.

### Create page (`/`), edit phase

Editor first: no visible heading or lede before the form, an `h1.cb-sr-only` with
`create.heading`.

1. **Title input** (`#create-title`, `.cb-create-title`): borderless, at the view title's size
   (1.5rem, weight 700), a line under it only while focused; its label is screen-reader-only.
2. **Editor frame** (`.cb-editor-frame[data-editor]`, 12 px radius) holding the head
   (`[data-editor-head]`, sticky: tab bar, "All N", the add button; then type, name, language and the
   item actions — Write/Preview switch `.cb-create-switch` for Markdown, "Remove"), the item panel
   (textarea, preview or file drop) and the foot with the size display. The frame shows the one
   focus ring (`:focus-within`); the textarea has no border or outline of its own.
3. **Add button** (`create.addItem`, "+ Add item") stays right of "All N" (also when "All N"
   shows). At `MAX_ITEMS` (16) it is disabled and reads `create.itemsFull` ("16 of 16").
4. **Size display:** a decorative SVG bar (`[data-size-bar]`, `aria-hidden`) whose fill
   (`[data-size-fill]`) has `width` = used / `MAX_ITEM_BYTES` in percent (capped at 100, one
   decimal), and the text `create.size`/`create.sizeOver`, which stays the accessible
   information. Over the limit both turn danger-coloured (`.cb-size-bar-over`,
   `.cb-size-meter-over`) and the submit button is disabled.
5. **Options row:** expiry, burn and password side by side from 720 px, each label above its
   control (the burn checkbox's label above it is `create.burnLabel`, the checkbox text
   `create.burn`). The password hint (`create.passwordNote`, `[data-password-note]`) is hidden
   while the password field is empty and shown once it has content.
6. **Submit row:** the submit button (lock icon) at its content width, the trust line
   (`create.lede`, lock icon) next to it; below 34rem the button takes the form width.

The draft logic (`hasDraft`, store-only language switch, "Back to editing" restoring the `Draft`)
is unchanged by the layout.

### Sealed page (after a successful create)

`tmpl-done`, filled by `showDone(shareUrl, deleteUrl, draft)`:

- the `sealed` hexagon next to `h1` "Sealed." (`create.done`) and a summary chip row
  (`[data-summary]`): `create.summary.items` ("{count} items · {size}", plural), the expiry
  (`formatExpiry`, "No expiry" without one), `create.summary.password` or `.noPassword`, and
  `create.summary.burn` if set. Built from the submitted `Draft` (non-empty items, their byte
  sizes, expiry measured from now), never from the server's answer.
- the **link panel** (`.cb-link-panel`): the share link as text in `#share-link[data-share-link]`
  (no `input`; monospace, wraps anywhere, `user-select: all`), the part from `#` on in its own
  `[data-share-key]` in the accent colour; the note `create.keyNote` ("the highlighted part");
  one filled "Copy link" button (`[data-copy="share-link"]`) — the only accent background on the
  page.
- the **delete-link row** (`[data-delete-row]`, dashed `border_strong` border): `create.deleteLink`,
  the management URL as text in `#delete-link[data-delete-link]`, `create.deleteLinkNote`, and a
  small ghost "Copy" (`[data-copy="delete-link"]`).
- the "Create another" link.

The text of `[data-share-link]` and `[data-delete-link]` is exactly the share and the management
URL; the browser tests read the links from there.

### Create states, manage page

- **Create:** `tmpl-encrypting` (`running`) and `tmpl-error` (`empty`) put the hexagon into their
  `[data-hexagon-slot]` (`placeHexagon`).
- **Manage:** every phase template wraps its content in `.cb-state`; `showState(template, state)`
  prepends the hexagon: loading `running`, confirmation `warning`, deleting `running`, deleted and
  unavailable `empty`. The confirmation shows the facts as chips (burn chip first,
  `.cb-chip-warning` when the capsule burns; expiry; ciphertext size), the irreversibility as a
  muted note (`.cb-state-note`), then `[data-delete-btn]` as `.cb-btn.cb-btn-danger` (outlined:
  `danger` text and border on `surface`, `tint_danger` on hover) and a ghost "Cancel".
- State layout (`states.css`): `.cb-state` is a centred column (hexagon, heading, text, actions
  in `.cb-state-actions`), at most 34rem wide, without a frame.

### Contrast

The design puts coloured text on the tint tokens (chips, the copied state, the danger hover, the
warning states). `tests/fixtures/contrast-pairs.json` therefore also requires 4.5:1 for `accent` on
`tint_accent`, `danger` on `tint_danger`, `warning` on `tint_warning`, `success` on `tint_success`
and `text` on each tint, in all four palette blocks; the built-in palette passes (lowest: `warning`
on `tint_warning`, light, 4.57:1). The same pairs drive the operator colour warnings
([branding.md](branding.md#theme-colours)).

## Structure

| Where | What |
| --- | --- |
| `src/server/web/src/pages/icons.ts` | `icon(name)`: inline SVG icons as DOM (`copy`, `check`, `download`, `chevron-left/-right/-down`, `remove`, `lock`, `clock`, `stack`, `theme`, `sun`, `moon`), 16 × 16, `aria-hidden`, `data-icon`; `drawing()` builds any `<svg>` from shapes (used by the hexagon) |
| `src/server/web/src/pages/hexagon.ts` | `hexagon(state)`, `HexagonState` |
| `src/server/web/src/pages/copy-state.ts` | `createLiveRegion()`, `CopyFeedback` (`copy`, `show`, `restore`), `COPIED_FOR_MS` |
| `src/server/web/src/pages/view.ts` | Display phase: `#renderDisplay` (title, `#renderChips`, sheet), `#renderTabs`, `#renderAllMenu`, `#renderItemNav`, `#renderItemActions`, `#fitTabs`; hexagon in every other phase; `#hexagon`/`#icon` hand Lit the same SVG node per place and state or icon on every render (`#drawings`) |
| `src/server/web/src/pages/open-capsule.ts` | `burnAfterRetrieval` passed from the descriptor into the outcome |
| `src/server/web/src/pages/render-items.ts` | Item content without card; `.cb-code` with the gutter (`lineNumbers`) |
| `src/server/web/src/pages/create.ts` | `showEdit` (icons, password hint), `layoutTabs`/`menuLine`/`removeControl` ("All N" with remove controls), `render` (add button `create.addItem` / `create.itemsFull` "16 of 16"), `refreshSizeMeter` (bar), `showDone`/`summaryParts`/`showShareLink`, `placeHexagon` |
| `src/server/web/src/pages/manage.ts` | `showState(template, state)` for every phase |
| `src/server/web/src/pages/shell.ts` | Imports the stylesheets (`tokens`, `base`, `view`, `create`, `states`); `mountLanguageSwitch` (quiet buttons) |
| `src/server/web/src/theme.ts` | `mountThemeToggle`: icon per choice, name `theme.switch`, choice as `title` |
| `src/server/web/src/styles/tokens.css` | `--cb-radius-control`, `--cb-radius-surface`, `--cb-duration-quick` |
| `src/server/web/src/styles/base.css` | Chrome (`.cb-head`, `.cb-foot`, `.cb-lang`), reduced-motion rule, `.cb-quiet`, `.cb-icon`, `.cb-bar-label`, `.cb-copied`, `.cb-chips`/`.cb-chip`/`.cb-chip-warning`, `.cb-sheet`/`.cb-sheet-bar`, `.cb-bar-actions`, `.cb-editor-frame`, `.cb-link-panel`, menu rows, `.cb-itemtab`, `.cb-btn-danger`, transitions, `.cb-hex*` |
| `src/server/web/src/styles/view.css`, `create.css`, `states.css` | One stylesheet per page area: view sheet, chips, switch, gutter, previous/next; editor, options row, size bar, sealed page; `.cb-state` layout |
| `src/server/web/{index,p,manage,report,legal}.html` | Header with `.cb-lang` and the SVG theme toggle; `index.html` templates `tmpl-edit`, `tmpl-encrypting`, `tmpl-done`, `tmpl-error`; `manage.html` templates in `.cb-state` |
| `src/server/web/src/i18n/{en,de}.json` | `items.all`, `items.position`, `items.previous`, `items.next`, `view.decryptedLocally`, `view.deletedAfterRetrieval`, `shell.copied`, `shell.copiedToClipboard`, `create.itemsFull`, `create.burnLabel`, `create.done`, `create.keyNote`, `create.copyLink`, `create.deleteLinkNote`, `create.summary.*` |

### Tests

| Test | Covers |
| --- | --- |
| `tests/icons.test.ts`, `tests/copy-state.test.ts`, `tests/theme.test.ts` | Icons and hexagon markup; copied state with fake timers (restore, second copy, refused clipboard, live region); toggle icon and name |
| `tests/view-page-display.test.ts`, `tests/render-items.test.ts`, `tests/item-tabs.test.ts` | Sheet structure, chip order for burn/no burn × one/many items, "All N" list and keyboard, previous/next, copy; the running hexagon and the chip/action icons stay the same nodes across a re-render or tab switch; gutter numbers, unchanged `<code>` text, no gutter for plain text and Markdown source |
| `tests/create-page.test.ts`, `tests/manage-page.test.ts` | Editor, size bar, password hint, "All N" with remove controls, "16 of 16", sealed page (link, key, summary, copy), hexagon per state |
| `tests/shells.test.ts` | SVG theme toggle without text in every shell, no `◐` |
| `tests/contrast.test.ts` | The tint pairs on all four palette blocks |
| `tests/catalogue-references.test.ts` | Every catalogue key is referenced by a source file, shell template or head script (replaced texts are removed, not left behind); the replaced "more"/"copied" keys are gone; `create.keyNote` names no colour |
| `e2e/acceptance.e2e.ts` | No border or shadow inside the item panel for every item kind (Markdown source too); burn chip first in the warning colour in both schemes, chip row between title and sheet; code gutter `aria-hidden`, not selectable, outside `<code>`; item actions with icon and label at 1280 px |
| `e2e/layout.e2e.ts` | Quiet header/footer, radii, transitions and reduced motion, 400 px, accent only on the active tab and the primary action, reading measure, editor-first create page |
| `e2e/items.e2e.ts` | 16 items on view and create: "All N", previous/next, remove from the list, "16 of 16" |
| `e2e/states.e2e.ts` | Sealed page and every state's hexagon with and without reduced motion, copied state, opening/decrypting/password/burn flows |
| `e2e/guard.ts` | Additionally fails on any inline `style` attribute, also one set through the CSSOM |

Computed styles in the browser tests are compared through `e2e/style.ts` (colours normalised by a
canvas context, the same in every engine).

## Decisions

- **Content as the focus, the accent reserved** — the accent marks only the active tab and the one
  primary action of a page (the sealed page's "Copy link"); switches, the language switch and bar
  actions are quiet. Several accent elements of equal weight were what made the earlier pages read
  as boxes of equal importance.
- **Language switch: two quiet text buttons, not one toggle button** — with two languages both
  names side by side are clearer, and the existing `[data-language-switch]` behaviour and browser
  tests stay unchanged.
- **Theme toggle as inline SVG** — the `◐` glyph rendered as a speck where the installed fonts
  lacked it; an SVG looks the same everywhere and needs no font. The shell carries the "auto" icon
  so the button is never empty before the script runs.
- **Radii as role tokens, not colour tokens** — not part of `[branding.colors]`; operators set
  colours, not layout.
- **Line numbers as a separate gutter**, generated from the line count, not by splitting
  highlight.js output per line — multi-line tokens stay intact, copying and selecting are
  unaffected, and the highlighter's allowlist (ADR-006) is untouched.
- **"All N" lists every item, but appears only on overflow** — a full list in fixed order puts an
  item at the same place at every width; showing the button when everything fits would add a
  control without a purpose. Horizontally scrolling tabs and a sidebar list were rejected.
- **Previous/next only on the view page** — it serves a reading flow; the create page has the
  editor in its place.
- **One hexagon builder, one mapping for every state page** — every progress state `running`,
  every irreversible confirmation `warning` (burn warning and delete confirmation alike), every
  terminal state `empty`; the symbol signals a state, so it stays on an instance with an operator
  logo.
- **View page SVG nodes built once per place** — `hexagon()` and `icon()` return new nodes, and
  Lit replaces a node it is handed anew; any re-render (a tab switch, for example) would then restart the `running` hexagon's dash. The view page keeps one node
  per place and state or icon and hands Lit that node again; the key includes the place because a
  node can have only one parent.
- **All motion in CSS** (`stroke-dashoffset`, transitions) in bundled stylesheets — the CSP allows
  no inline style, and the reduced-motion preference is honoured in one place.
- **Size bar as a decorative SVG next to the text, not a `<meter>`** — a `<meter>` cannot be
  styled reliably across engines without inline styles; the fill's `width` is a presentation
  attribute, which the CSP and the inline-style guard allow. The text stays the accessible
  information.
- **Sealed summary from the submitted `Draft`, never from the server response** — it describes
  what the creator chose; the server's answer is not the source of capsule facts.
- **Share link as text with the key part highlighted, the key note saying "highlighted"** — the
  full link shows what is shared; the note names no colour because operators can change the
  accent.
- **Danger button outlined, not filled** — a filled danger button would need a text colour on
  `danger` that the contrast pairs do not cover, and no colour token is added; `danger` on
  `surface` and on `tint_danger` are covered.
- **Copied state parks the button's own nodes instead of rebuilding them** — Lit owns the view
  page's button content; replacing it would break Lit's later updates.
- **Tint pairs in the contrast fixture** — the new chips, key highlight, copied state and warning
  states put coloured text on the tint tokens; operators with `[branding.colors]` may get a warning
  for these pairs, never an error.
- **One stylesheet per page area** (`view.css`, `create.css`, `states.css`) next to the shared
  components in `base.css`.
- No ADR: CSP, colour tokens, the MPA structure (ADR-004), the theme mechanics (ADR-005) and the
  branding rules (ADR-017) are unchanged.

## Limits and open points

- No QR code on the sealed page; no line numbers for plain text or Markdown source.
- The legal and report pages only take the shared chrome and control styles; they have no own
  redesign.
- The danger button deviates from the mockup (outlined instead of filled), see above.
- WebKit runs the browser tests only in CI; locally they are run in Chromium and Firefox.
- A visual comparison against the design mockups in light, dark and at 400 px is a manual check.

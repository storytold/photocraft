# Dialogs

Modal parameter windows hosted by the craft framework's `DialogHost`. They mirror Photoshop: no
dimmed backdrop, canvas pan/zoom under an open dialog, Tab cycles text and number fields only, Esc
cancels, Enter confirms when OK is enabled. Automation drives the same field map as the UI
(`ui.dialog.*` in [`control-protocol.md`](control-protocol.md)).

## Architecture

| Piece | Location | Role |
|---|---|---|
| Hosted-dialog glue | `crates/ui-egui/src/dialogs.rs` | `run`, `take_results`, `TAKERS`, `PROMPTS`, `PARAM_FAMILIES`, `confirm`, `cancel`, `open_command_dialog`, `move_memories` |
| Dialog host | `app.framework.dialogs` (`DialogHost`) | Draws every open dialog, stacks them, routes OK, Cancel and Esc, keeps the results until taken |
| Chrome | framework `DialogFrame` | Title, close box, footer buttons in platform order, `FocusPolicy::DIALOG` with `initial: FirstField` |
| Parameter dialogs | framework `HostedParamDialog` over `ParamDialog` | Rows from a `Param` list, remembered values, Alt+Cancel → Reset (`alt_resets`, `reset_to`), own undo history |
| Prompts | framework `StandardPrompt` | Question with Proceed / Alternate / Cancel and "Don't show again" |
| Menu / command entry | `dialogs::open_command_dialog` | Picks a dedicated opener, the adjustment editor, the schema dialog, or a bare command form |
| Filters | `filter_dialog.rs` | Schema from engine params, live preview, the generic family |
| Adjustments | `adjust_dialog.rs` | Shared editors with Properties; preview via `adjust_preview` |
| Family modules | one module per dialog family | `open`, and either `owns` + `run` (a parameter family), `answer` (a prompt) or `take` (its own results) |

**Frame.** `dialogs::run(app, ui)` runs once per pass. It syncs the guide edit, shows the command
palette, feeds the filter preview, runs the host, then calls `take_results`. The host applies only a
pass's own input and every result is taken once, so a second pass in one frame acts on nothing twice.

**Results.** `take_results(app, ctx)` runs after every host pass and at once after `confirm`,
`cancel` and an automation `ui.dialog.set`, so an answer acts inside the request that caused it.
In order it runs:

1. `params::take`: each finished `HostedParamDialog` goes to the first `PARAM_FAMILIES` row whose
   `owns(key)` is true. OK calls the row's `run`; Cancel calls its `cancel` hook. Then held OKs and
   the dialogs' requests are answered (see below).
2. `prompts_take`: each finished `StandardPrompt` answer goes to the `PROMPTS` rows in order until one
   returns true. Only an `Ok` carries an answer; a prompt's own Cancel button answers `Ok` with
   `PromptChoice::Cancel`. The host keeps one plain result per dialog type, so only `prompts_take`
   takes `StandardPrompt` results.
3. Each `TAKERS` entry: the family modules that take their own dialog types.

**Field contract.** A dialog's values are its `DialogFields`, keyed by parameter. Keys never start
with `__`. Automation sets them through `DialogHost::set_json_field`.

### Parameter families (`dialogs::params`)

| Item | Role |
|---|---|
| `ParamFamily` | `owns`, `run`, and optional `check`, `listing`, `footer`, `cancel`, `document` |
| `PARAM_FAMILIES` | Every family in match order; the generic filter family is last and owns every engine command |
| `open_param(app, key, fields, captured, build)` | Opens the dialog with history, remembered values under `photocraft.<key>` and the active document's canvas for droppers; `captured` is the per-open state the family reads with `params::captured` and `take` drops |
| `answer_held` | Runs a family's `check` on a held OK and answers the dialog; a refusal shows under the rows |
| `answer_requests` | Colour rows open the Color Picker as a child (`color_picker_ui::open_for_request`) and its OK comes back through `answer_for`; Browse rows pick through `host.files()`; droppers arm through `canvas_input`; files rows ask the family's `listing`; footer buttons go to `footer` |

`confirm(app, ctx, id)` answers a dialog with OK and takes the results; a refused held OK comes back
as the error text. `cancel(app, ctx, id)` cancels and takes the results. `cancel_for_closed` cancels
the dialogs whose family reports a closed document from the captured value.

### Remembered values

A parameter dialog remembers its values in the `remembered` settings section under
`photocraft.<key>` (`Remembered`). `move_memories(prefs, host)` runs once before the first frame and
moves the old `Preferences.dialogs` entries there. It applies three parts in order:

1. **Keep**: the keys the engine reads (`ui.layersPanelOptions`, `file.print`, the Save for Web
   memory) and every don't-show-again key.
2. **Route**: a key a library crate names (`filter.cameraRaw.scope`, `image.adjustments.<kind>`,
   `shape.create.<kind>`).
3. **Generic**: every other entry moves to `photocraft.<key>`, nulls dropped.

### Command routing

`open_command_dialog(app, command, label)` returns the new dialog's id or the opener's refusal text.
First match wins:

| Command | Opener |
|---|---|
| `view.newGuideLayout` | `guide_layout_ui::open` |
| `edit.stroke` | `stroke_ui::open` |
| Equalize, when it asks | `equalize_ui::open` |
| New Group fronts | `new_group_ui::open` |
| `select.colorRange` | `color_range_ui::open` |
| `slice.set`, `slice.divide` | `slice_ui::open_options`, `slice_ui::open_divide` |
| `image.adjustments.*` | `adjust_dialog::open` |
| commands with a parameter schema | `filter_dialog::open` |
| anything else | a bare command form that the generic filter family runs |

Other dedicated families open from their menu, tool or panel code with their own `open`.

## Parallel modal systems

| System | Location | When |
|---|---|---|
| Unsaved changes | `discard_ui.rs` | Close, Close Others/All, Exit—Save / Don't Save / Cancel, where on macOS ⌘D is Don't Save too (#11282); Revert—Revert / Cancel, with no Don't Save for ⌘D to press |
| Shell dialog | `control.rs`, `ui.inspect` id `"shell"` | Workspace, custom pixel aspect, show-extras options |
| Camera Raw | `camera_raw_ui.rs` | Separate OS window on desktop; see control-protocol § Camera Raw |
| Nested pickers | `color_picker_ui` | Open as a child of the dialog that asked; the parent closes them |

## Rules

### Architecture and commands

1. **Thin UI, engine command on OK.** A family's `run` calls the engine command; one history step unless the command itself batches.
2. **Register in the glue.** A parameter family adds a `PARAM_FAMILIES` row; a prompt adds a `PROMPTS` row; a module with its own dialog type adds a `TAKERS` row. Add the opener to `open_command_dialog` when it is menu-driven.
3. **Extend existing paths first.** Filters → `filter_dialog`; adjustments → `adjust_dialog`; a command form with rows → `ParamDialog::command`.
4. **The field map is the automation contract.** Anything agents must drive lives in named fields, not in local egui state.
5. **Remember last-used values** through `ParamResult::remember` after the command succeeds.

### Chrome and layout

6. **Footer buttons** come from the framework `DialogFrame`: platform order, the default action in the corner.
7. **Dialogs are sized from their content** by the framework.
8. **Transparent backdrop**: no dimming; previews are judged at true contrast.
9. **Placement** is remembered by the framework; the first open is centred.
10. **Stacking:** only the top dialog is interactive; a child opens over its parent and closes with it.

### Keyboard and focus

11. **Tab / ⇧Tab** move between text and number fields (`FocusPolicy::DIALOG` with `initial: FirstField`), with select-all on keyboard arrival.
12. **Open on first value:** value-first dialogs focus their first field.
13. **Enter** confirms when OK is enabled.
14. **Esc** on the top dialog with no popup open cancels. **Click outside does not dismiss.** Esc ignores Alt.
15. **Alt** turns Cancel into **Reset** when the dialog has `alt_resets`; Reset restores `reset_to` or the opening values.

### Canvas and preview

16. **Live preview** for filters and adjustments uses `filter_preview` / `adjust_preview`; a family's Cancel handling clears it.
17. **Under-dialog canvas:** a dialog with `ParamDialog::canvas` keeps the canvas view usable under the modal layer; droppers sample it through `canvas_input`.

### Labels and i18n

18. **Titles:** command forms use the command's catalogued label; fixed dialogs use `tl!`.
19. **User-visible strings:** `tl!("…")`.

### Confirm / cancel safety

20. **Child dialogs first:** a parent's OK is refused while a child picker is open; cancelling a parent closes its children.
21. **Validate before close** through the family's `check`.
22. **Disable OK** when required input is missing.
23. **Cancel side effects** (guide edit, live preview, Actions record) run from the family's `take` or `cancel` hook on a Cancel result.

### Testing

24. **egui_kittest:** open the dialog, `run_steps`, press keys, assert the dialog's fields. `dialogs::testing::{confirm, cancel}` answer a dialog from a test.
25. **Filter preview:** `dialogs/tests.rs` covers preview persistence across background jobs.

### Anti-patterns

26. No panics in `run` or `check` on bad input; return the refusal.
27. No hard-coded colours or radii; use the framework theme tokens and components.
28. Do not skip `take_results` after a programmatic confirm or cancel.
29. Do not expect click-outside-to-close.

## Adding a command dialog (checklist)

1. Choose the path: `adjust_dialog`, `filter_dialog`, a bare command form, or a new family module.
2. `open`: build the `DialogFields` and call `params::open_param(app, key, fields, captured, build)`.
3. `run(app, ctx, result)`: call the engine command, then `result.remember(ctx)`.
4. Add the `PARAM_FAMILIES` row (before the generic filter family) and, when menu-driven, the `open_command_dialog` arm.
5. Cancel work (preview, record): set the row's `cancel` hook.
6. Tests: open, set fields through `set_json_field`, `dialogs::testing::confirm`, assert the engine or family state.

## Known gaps

- **“Dialog module”** is any UI whose family is routed from `dialogs.rs`, not only files named `*dialog*`.
- **Numeric field parity** (units in expressions, scrubby labels) is tracked in [`ui-parity.md`](ui-parity.md) (Navigation / numeric fields), not duplicated here.

## Revision history

| Date | Change |
|---|---|
| 2026-11-06 | New Guide / Edit Selected Guides: Position Tab focus and exact edit-position handling |
| 2026-10-11 | Initial doc from dialog codebase audit |

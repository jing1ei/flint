# Custom Skins

Open **Flint > Customize Skin...** from the native menu, or click the palette
button in the Settings header. The shortcut is Cmd+Shift+, (Ctrl+Shift+, in the browser preview).

1. Click **Copy LLM prompt** for the current skin and its editing rules, or **Copy code** for JSON alone.
2. Paste into your preferred LLM and replace the preference placeholder with your desired look.
3. Paste the returned JSON into **Skin code**. A single fenced JSON block is accepted too.
4. Click **Preview skin**, then **Keep skin** within 20 seconds. Otherwise the previous skin returns.

**Revert**, leaving the editor, or closing Settings cancels an unconfirmed preview.
**Reset skin** restores the original appearance without resetting conversion preferences.
The editor itself always uses the original light palette and fonts, so even an unreadable imported
palette cannot hide its recovery controls. The queue stays visible behind it during preview.

## Scope

Skins contain `version: 1`, a name, complete `light` and `dark` color palettes, and three local
font-family stacks: `sans`, `serif`, and `mono`. The default is light pink-blue in either OS appearance. Custom skin palettes follow the operating system.
Copying the original skin gives the complete schema and current token names.

Only existing color variables and font-family variables are applied. No CSS, JavaScript, HTML,
URLs, remote font loading, selectors, layout rules, font sizes, weights, spacing, animations,
conversion settings, file paths, or commands are accepted. Unknown fields reject the entire skin.
Colors accept six/eight-digit hex or numeric `rgb()`/`rgba()` notation.

Font names may use letters, digits, spaces, underscores, and hyphens. Use fonts installed on your
machine, with generic fallbacks such as `sans-serif`, `serif`, or `monospace`. Font-family changes
change glyph metrics and may affect text wrapping; the app's sizes, spacing, grid, and layout rules
remain unchanged. Preview the result before keeping it.

The app does not contact an LLM or send anything automatically. Clipboard exports contain only
skin data and, when requested, the editing prompt. Saved skins live in the webview's local storage,
under `cross-converter.skin.v1`, independently of Rust conversion settings. Browser preview and
the desktop app have separate storage. Invalid saved data falls back to the original skin.

## Verification

`npm test` covers schema validation, restricted CSS output, timeout/revert, persistent-storage
failure, and separation from conversion preferences. `python3 scripts/ui_skin.py` exercises copying,
LLM prompt export, imports, preview/keep, reload, reset, geometry checks, and a mock conversion at
960px/light and 720px/dark.

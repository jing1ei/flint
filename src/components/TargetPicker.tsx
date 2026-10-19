/**
 * The output format control for one row (and for the bulk "convert all to" header).
 *
 * Suggested targets come first, everything else the category can write follows in a second group;
 * formats whose helper is missing stay visible but disabled, with the missing tool in the label —
 * seeing *why* something is unavailable beats wondering where it went.
 *
 * The list is built when the control is *used*, not when the row appears. A queue's worth of
 * pickers is a queue's worth of option lists — a little over twenty `<option>` elements each — and
 * five thousand rows made a hundred thousand of them: most of the document, most of the heap, and
 * the reason a long queue converted at eight frames a second. A closed `<select>` paints exactly one
 * option, so until it is opened that is all it holds; the first focus, pointer or keystroke builds
 * the rest, synchronously, before the browser has drawn the popup or moved the selection.
 */
import { useMemo, useState } from "react";
import { flushSync } from "react-dom";
import { optionLabel } from "../lib/format";
import type { CategoryId, CategoryView, FormatView } from "../lib/types";
import { useStore } from "../state/store";

interface Props {
  category: CategoryId;
  suggested: string[];
  value: string;
  disabled?: boolean;
  label: string;
  onChange: (target: string) => void;
}

/** Frozen, so an unopened picker's `useMemo` hands back the same pair every render. */
const NO_OPTIONS: [FormatView[], FormatView[]] = [[], []];

/**
 * The one format a closed picker has to know about: whatever it is currently set to.
 *
 * A plain scan rather than the `flatMap` the full list is built from — the closed state needs one
 * label, and allocating a flattened copy of the catalog per row to find it is the sort of thing that
 * only shows up at five thousand rows.
 */
function findOutput(categories: CategoryView[] | undefined, id: string): FormatView | undefined {
  if (categories === undefined) return undefined;
  for (const category of categories) {
    for (const format of category.outputs) if (format.id === id) return format;
  }
  return undefined;
}

export function TargetPicker({
  category,
  suggested,
  value,
  disabled = false,
  label,
  onChange,
}: Props): React.JSX.Element {
  const categories = useStore((s) => s.catalog?.categories);
  /** Has anybody asked this picker for its list yet? Once true it stays true. */
  const [opened, setOpened] = useState(false);

  const [quick, rest] = useMemo<[FormatView[], FormatView[]]>(() => {
    if (!opened) return NO_OPTIONS;
    const all = categories ?? [];
    // Suggestions cross category lines on purpose — a video's chips are mp4/webm/mov/**gif**/**mp3**,
    // and Flash is input-only, so its outputs live entirely in other categories. Resolving them
    // against this row's category alone would silently drop "video → GIF" and leave a .swf row with
    // an empty dropdown.
    const catalogOutputs = all.flatMap((c) => c.outputs);
    const picks = suggested
      .map((id) => catalogOutputs.find((f) => f.id === id))
      .filter((f): f is FormatView => f !== undefined);
    const shown = new Set(picks.map((f) => f.id));
    const own = all.find((c) => c.id === category)?.outputs ?? [];
    return [picks, own.filter((f) => !shown.has(f.id))];
  }, [opened, categories, category, suggested]);

  // A target that is not in this category's output list (should not happen) still needs a slot,
  // otherwise the select would silently show something the row is not actually set to.
  const selected = opened
    ? (quick.find((f) => f.id === value) ?? rest.find((f) => f.id === value))
    : findOutput(categories, value);
  const known = selected !== undefined;
  const fallback = value === "" ? "—" : value;
  // The string the closed select paints. A <select> at `width: auto` sizes itself to its *widest*
  // option, which is what left the chevron 60px away from a label like "PDF"; sizing the wrapper to
  // the chosen label instead keeps the chevron against the word it belongs to. The select itself is
  // still the control — same element, same aria-label, same keyboard behaviour.
  const shown = selected !== undefined ? optionLabel(selected) : fallback;

  /*
   * Build the list, now, in this event.
   *
   * `flushSync` because the gesture that primes the picker is the gesture that reads it: Chromium
   * opens the popup as the default action of the same `pointerdown`, and moves the selection as the
   * default action of the same `keydown`. A state update left to React's own scheduling would land
   * after the browser had already looked, and the first arrow key on an untouched row would go
   * nowhere. Both are discrete events, so this is a flush React was about to do anyway — it is
   * spelled out because the correctness of the control depends on it, not on the scheduler.
   */
  const open = (): void => {
    if (opened) return;
    flushSync(() => setOpened(true));
  };

  return (
    <span className="target">
      <span className="target__sizer" aria-hidden="true">
        {shown}
      </span>
      <select
        className="target__select"
        aria-label={label}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.currentTarget.value)}
        // Every way in: the keyboard arrives at `focus` (Tab, or a programmatic focus) before it can
        // press anything, `keydown` covers a focus this component never saw, and `pointerdown`
        // precedes both the popup and the focus a mouse causes.
        onFocus={open}
        onPointerDown={open}
        onKeyDown={open}
      >
        {opened ? (
          <>
            {!known && <option value={value}>{fallback}</option>}
            {quick.length > 0 && (
              <optgroup label="Suggested">
                {quick.map((format) => (
                  <option key={format.id} value={format.id} disabled={!format.available}>
                    {optionLabel(format)}
                  </option>
                ))}
              </optgroup>
            )}
            {rest.length > 0 && (
              <optgroup label="All formats">
                {rest.map((format) => (
                  <option key={format.id} value={format.id} disabled={!format.available}>
                    {optionLabel(format)}
                  </option>
                ))}
              </optgroup>
            )}
          </>
        ) : (
          /* The closed box, and nothing else: the one option it is painting, disabled exactly as it
             would be inside the full list so that opening the picker changes nothing about it. */
          <option value={value} disabled={selected !== undefined && !selected.available}>
            {shown}
          </option>
        )}
      </select>
    </span>
  );
}

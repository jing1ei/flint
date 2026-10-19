/** Click or drop files on the mat. Touch guidance stays visible without hover. */
import { useEffect, useRef } from "react";
import { engineMissing, useStore } from "../state/store";

export function DropZone(): React.JSX.Element {
  const openFiles = useStore((s) => s.openFiles);
  const dragging = useStore((s) => s.dragging);
  const busy = useStore((s) => s.busy);
  const heldFiles = useStore((s) => s.hasHeldFiles);
  /*
   * The one exception to "not a word of copy at rest".
   *
   * With the bundled engine gone the app converts nothing, and the only place that said so was the
   * settings sheet — which a first-run user has no reason to open, so the first they learned of it
   * was every file they dropped failing. A healthy install still says nothing at all: this is a
   * statement of fact that is false almost always, not a status line.
   */
  const missing = useStore(engineMissing);
  const surface = useRef<HTMLDivElement>(null);

  /*
   * Where the keyboard goes when the queue is emptied.
   *
   * ⌫ on the last row, or ⇧⌘⌫ on the lot, takes the element the keyboard was standing on with it,
   * and the browser's answer to that is `<body>`: the ring vanishes and the next Tab starts again at
   * the top of the window. This surface is the only thing left to stand on, and it is the control
   * that undoes what just happened.
   *
   * Never on a first launch (`hasHeldFiles`), where nothing has been taken away from anybody and the
   * canvas is meant to be untouched — and never over a focus that is still somewhere real.
   */
  useEffect(() => {
    if (!heldFiles) return;
    if (document.activeElement !== document.body) return;
    surface.current?.focus();
  }, [heldFiles]);

  const choose = (): void => {
    if (busy) return; // a second dialog while paths are being inspected would race the first
    void openFiles();
  };

  return (
    <>
      <div
        ref={surface}
        className="dropzone"
        role="button"
        tabIndex={0}
        aria-label="Choose files to convert"
        aria-busy={busy || undefined}
        // While paths are being inspected the surface ignores clicks; say so instead of looking live.
        aria-disabled={busy || undefined}
        data-dragging={dragging || undefined}
        data-busy={busy || undefined}
        onClick={choose}
        onKeyDown={(event) => {
          if (event.key !== "Enter" && event.key !== " ") return;
          event.preventDefault();
          choose();
        }}
      >
        {/*
          The mat. A hairline rectangle inset from the window edge is what turns the emptiness into
          negative space: it declares the drop surface and gives the "+" a plate to sit on. It is
          also the focus ring and the drag indicator, so the plate reacts as one material.
        */}
        <span className="dropzone__frame" aria-hidden="true" />

        <span className="dropzone__figure">
          {/*
            An engraved mark, not a watermark: 36px, one pixel thick, equal arms, butt caps. The
            strokes sit on half-pixel coordinates inside an even-sized box so the 1px line lands on
            one whole device pixel instead of blurring across two — at this size that is the whole
            difference between "fine" and "smudged". `non-scaling-stroke` keeps it 1px if the box
            is ever scaled.
          */}
          <svg className="dropzone__plus" viewBox="0 0 36 36" aria-hidden="true" focusable="false">
            <line x1="18.5" y1="0.5" x2="18.5" y2="35.5" vectorEffect="non-scaling-stroke" />
            <line x1="0.5" y1="18.5" x2="35.5" y2="18.5" vectorEffect="non-scaling-stroke" />
          </svg>
          <span className="dropzone__hint" aria-hidden="true">
            Click to choose&#8195;·&#8195;or drop&#8195;·&#8195;or paste a link
          </span>
        </span>
      </div>

      {/*
        The broken-install line, in the mat's own bottom margin: absolutely positioned against
        <main>, so the plate above it is composed exactly as it is on a working install, and
        `pointer-events: none` so the whole window is still one click target underneath it.

        A sibling of the surface rather than a child of it, because the surface is a `role="button"`
        and a screen reader will not walk into one — the only warning this app has would have been
        unreachable to exactly the users least able to guess why nothing converts.
      */}
      {missing ? (
        <span className="enginenote" role="status">
          FFmpeg is missing&#8195;·&#8195;nothing can be converted
        </span>
      ) : null}
    </>
  );
}

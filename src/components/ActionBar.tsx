/**
 * The bottom bar: a hairline rule, one line of muted micro-copy on the left, and the single
 * primary action on the right. Aggregate progress is a 1px line across the very bottom of the
 * window — the only other thing this bar is allowed to draw.
 *
 * It exists only while there are files; the empty canvas has no action bar at all.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, Crop } from "lucide-react";
import { CropDialog } from "./CropDialog";
import {
  etaSuffix,
  parentDir,
  pluralFiles,
  presetLabel,
  presetLine,
  summaryLine,
  trimSummary,
} from "../lib/format";
import {
  aggregateProgress,
  allRows,
  convertibleRows,
  destinationFolder,
  isTerminal,
  useStore,
  type Row,
} from "../state/store";

export function ActionBar(): React.JSX.Element {
  const [menu, setMenu] = useState(false);
  const split = useRef<HTMLDivElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);
  const item = useRef<HTMLButtonElement>(null);
  const setCropOpen = useStore((s) => s.setCropOpen);
  const files = useStore((s) => s.files);
  const order = useStore((s) => s.order);
  const batchIds = useStore((s) => s.batchIds);
  const phase = useStore((s) => s.phase);
  const inherited = useStore((s) => s.inheritedBatch);
  const stopping = useStore((s) => s.stopping);
  const summary = useStore((s) => s.summary);
  const settings = useStore((s) => s.settings);
  const catalog = useStore((s) => s.catalog);
  const destination = useStore((s) => s.destination);
  const start = useStore((s) => s.start);
  const stop = useStore((s) => s.stop);
  const open = useStore((s) => s.open);
  useEffect(() => {
    if (!menu) return undefined;
    item.current?.focus();
    const outside = (event: PointerEvent | FocusEvent): void => {
      if (event.target instanceof Node && !split.current?.contains(event.target)) setMenu(false);
    };
    const windowBlur = (): void => setMenu(false);
    document.addEventListener("pointerdown", outside);
    // WebKit blurs a focused menu item to the body on mousedown without focusing
    // the button. Closing on blur would unmount it before its click can open the dialog.
    document.addEventListener("focusin", outside);
    window.addEventListener("blur", windowBlur);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("focusin", outside);
      window.removeEventListener("blur", windowBlur);
    };
  }, [menu]);
  useEffect(() => { if (phase === "running") setMenu(false); }, [phase]);

  const rows = useMemo(() => allRows({ files, order }), [files, order]);
  const queued = useMemo(() => convertibleRows({ files, order }), [files, order]);
  const active = useMemo(
    () => [...batchIds].map((id) => files[id]).filter((row): row is Row => row !== undefined),
    [batchIds, files],
  );

  const progress = phase === "running" ? aggregateProgress(active) : 0;
  const done = active.filter((row) => isTerminal(row.status)).length;
  const etas = active
    .filter((row) => row.status === "running" && row.eta_secs !== null)
    .map((row) => row.eta_secs ?? 0);
  const eta = etas.length > 0 ? Math.max(...etas) : null;

  const outputFolder = useMemo(() => {
    const finished = rows.find((row) => row.status === "done" && row.output !== null);
    if (finished?.output == null) return null;
    // Same `""` as the store reads: a path with no folder in it has no folder to open, and
    // `open("")` is worse than no button at all.
    const folder = parentDir(finished.output);
    return folder === "" ? null : folder;
  }, [rows]);

  /*
   * The folder the next run writes into.
   *
   * `destinationFolder` rather than `parentDir(destination)` here: the slot holds an estimated
   * *file* path when there is a file to estimate from and `LinkSupport.destination` — already a
   * folder — for a queue of nothing but links, and only the store can tell the two apart, because
   * only the store can see the rows. Deciding it here by comparing the string against the last
   * `LinkSupport` answer named `/Users/you` under a queue of links the moment the two disagreed.
   */
  const savesTo = useMemo(
    () => destinationFolder({ files, order, destination }),
    [files, order, destination],
  );

  const label =
    phase === "running"
      ? stopping
        ? "Stopping…"
        : "Stop"
      : queued.length <= 1
        ? "Convert"
        : `Convert ${queued.length} ${pluralFiles(queued.length)}`;

  /*
   * What the next run will do, in one line: where the files land, and the cut if there is one.
   *
   * The trim rides *inside* the existing line rather than under it, because the bar's first tier is
   * a fixed box and a second sentence would change the window's height. It is in the idle copy and
   * nowhere else on purpose: this is the line about the run that is about to happen, and a
   * persisted setting that shortens every file is not something a user should have to open a sheet
   * to rediscover. Joined with the same ` · ` the rest of the bar separates facts with.
   */
  const idle = [savesTo === null ? "" : `Saves to ${savesTo}`, trimSummary(settings)]
    .filter((part) => part !== "")
    .join(" · ");

  /*
   * One line of status per phase, resolved here rather than as four sibling elements: the bar's
   * first tier is a fixed box that always exists, so its height cannot change when a batch ends
   * and there is never a phase in which the preset caption is left standing on its own.
   *
   * The countdown is deliberately *not* part of this string — see `countdown` below.
   */
  const status =
    phase === "running"
      ? inherited
        ? // A conversion the shell started before this page loaded: no rows here to count, and its
          // tally is not this window's to report. Say what is happening and offer Stop.
          "Finishing a conversion that started before this window opened"
        : `${done} of ${active.length} done`
      : phase === "finished" && summary !== null
        ? summaryLine(summary.ok, summary.failed, summary.skipped)
        : queued.length === 0 && rows.length > 0
          ? rows.some((row) => row.status === "done")
            ? "Converted files are ready. Add files to start another batch."
            : "Nothing in this list can be converted"
          : idle;
  /*
   * ` · 2m 30s left`, rendered inside the line but hidden from the accessibility tree.
   *
   * The whole note is a polite live region, and FFmpeg reports progress several times a second: with
   * the countdown in the announced text a screen reader was handed a new sentence every tick and
   * never finished one. Kept out of it, the region speaks once per file — "3 of 40 done" — while the
   * eye still gets the number that is worth watching.
   */
  const countdown = phase === "running" && !inherited ? etaSuffix(eta) : "";
  const showOpenFolder = phase !== "running" && outputFolder !== null;

  return (
    <footer className="actionbar">
      <div className="actionbar__note" aria-live="polite">
        <span className="actionbar__line">
          <span
            className="actionbar__text"
            title={phase === "idle" && destination !== null ? destination : undefined}
          >
            {status}
            {countdown !== "" && <span aria-hidden="true">{countdown}</span>}
          </span>
          {showOpenFolder && (
            <button
              type="button"
              className="microlink"
              // Opens the folder rather than revealing it: after a batch the useful thing is
              // the list of new files, not the `Converted` folder highlighted in its parent.
              onClick={() => void open(outputFolder ?? "")}
            >
              Open folder
            </button>
          )}
        </span>
        <span className="actionbar__preset" title={presetLine(catalog, settings)}>
          {presetLabel(catalog, settings)}
        </span>
      </div>

      <div ref={split} className="convert-split" data-options={phase !== "running" || undefined} onKeyDown={(event) => {
        if (menu && event.key === "Escape") {
          event.preventDefault(); setMenu(false); toggle.current?.focus();
        }
      }}>
      <button
        type="button"
        className={`pill${phase === "running" ? " pill--stop" : ""}`}
        // Without settings there is nothing to send to Rust, so `start` would return silently. That
        // happens when the initial `get_settings` failed: the toast says why, and a dead-looking
        // button is honest where a live one that does nothing is not.
        disabled={phase === "running" ? stopping : queued.length === 0 || settings === null}
        onClick={() => void (phase === "running" ? stop() : start())}
      >
        {label}
      </button>
      {phase !== "running" && (
        <button ref={toggle} type="button" className="convert-split__toggle" title="More conversion options"
          aria-label="More conversion options" aria-haspopup="menu" aria-expanded={menu}
          disabled={queued.length === 0 || settings === null}
          onKeyDown={(event) => { if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault(); setMenu(true);
          } }}
          onClick={() => setMenu((value) => !value)}>
          <ChevronDown size={16} aria-hidden="true" />
        </button>
      )}
      {menu && <div className="convert-split__menu" role="menu">
        <button ref={item} type="button" role="menuitem" onClick={() => {
          setMenu(false); toggle.current?.focus(); setCropOpen(true);
        }}><Crop size={15} aria-hidden="true" /> Crop &amp; convert</button>
      </div>}
      </div>
      <CropDialog />

      {/* Only when there is something to measure: an inherited batch has no rows here, and a bar
          frozen at 0% would be a worse lie than no bar at all. */}
      {phase === "running" && active.length > 0 && (
        <div className="actionbar__progress" aria-hidden="true">
          <div className="actionbar__progressfill" style={{ width: `${progress * 100}%` }} />
        </div>
      )}
    </footer>
  );
}

/**
 * One file: a typographic gutter carrying the source extension, the name, its metadata, and the
 * controls that only appear when they apply.
 *
 * No card, no box, no icon — a hairline underneath is enough separation. While the file converts,
 * a 1px accent line grows along the row's bottom edge; that is the entire progress treatment.
 *
 * A pasted link is the same row with three substitutions, not a second component: the gutter reads
 * `YT`/`BILI` because a URL has no extension, the name is the video's title once yt-dlp has resolved
 * it (and the short URL until then), and the status says which half of the job is running. Anything
 * that would have printed a fact a link does not have — its size on disk, its path — prints nothing.
 */
import { memo } from "react";
import {
  baseName,
  etaSuffix,
  extensionLabel,
  formatBytes,
  linkLabel,
  outputIsTimeBased,
  phaseStatus,
  stemOf,
  trimmedLength,
} from "../lib/format";
import {
  isTerminal,
  missingHelperFor,
  needsFullDiskAccess,
  rowProgress,
  signInFailure,
  useStore,
} from "../state/store";
import type { SignInFailure } from "../state/store";
import { TargetPicker } from "./TargetPicker";

/**
 * The verb for each sign-in ending — the walkthrough in every case, named for what it is *for*.
 *
 * "Set up" and "fix" are not interchangeable: a wall hit with nothing configured is a thing the
 * user has not done yet, while an unreadable jar or a refused cookie is a thing that is broken.
 * Telling somebody to fix what they never set up is how the old message lost people. The ellipsis
 * is macOS for "this opens something".
 */
const SIGN_IN_ACTION: Record<SignInFailure, string> = {
  wall: "Set up a sign-in…",
  unreadable: "Fix the sign-in…",
  refused: "Fix the sign-in…",
};

interface Props {
  id: string;
  /** Position in the list, used only to stagger the entrance a few milliseconds per row. */
  index: number;
}

/**
 * Memoised on `id`: a progress tick replaces the `files` map and re-renders the list, but only the
 * rows whose own slice changed need to paint - with a 200-file queue that is the difference between
 * a smooth progress bar and a stuttering one.
 */
export const FileRow = memo(function FileRow({ id, index }: Props): React.JSX.Element | null {
  const row = useStore((s) => s.files[id]);
  const running = useStore((s) => s.phase === "running");
  const inBatch = useStore((s) => s.batchIds.has(id));
  const selected = useStore((s) => s.selectedId === id);
  const select = useStore((s) => s.selectFile);
  const setTarget = useStore((s) => s.setTarget);
  const remove = useStore((s) => s.removeFile);
  const retry = useStore((s) => s.retry);
  const reveal = useStore((s) => s.reveal);
  const open = useStore((s) => s.open);
  const showPackage = useStore((s) => s.showPackage);
  const openFullDiskAccess = useStore((s) => s.openFullDiskAccess);
  const openSignInGuide = useStore((s) => s.openSignInGuide);
  // Selected as slices, not as a derived object: a selector that built one would hand
  // `useSyncExternalStore` a new snapshot on every call.
  const catalog = useStore((s) => s.catalog);
  const installPlans = useStore((s) => s.installPlans);
  // The trim is one setting for the whole batch, so every row reads the same slice of it.
  const settings = useStore((s) => s.settings);

  if (row === undefined) return null;
  const { info, link, status } = row;

  // True while the running batch owns this row: nothing about it may be changed or removed,
  // because Rust is already converting it and the UI would be describing a different job.
  const owned = running && inBatch && !isTerminal(status);

  /*
   * `Trimmed to 0:10` — what the batch's one cut will make of *this* row, when it makes anything.
   *
   * The row is the only place the trim stops being an abstract pair of numbers, so it is stated
   * where the source's own length already is, in the same wording. Three rows say nothing and are
   * meant to: a JPEG or a PDF, whose output has no timeline to cut (`outputIsTimeBased` asks the
   * same question `plan` asks, about the *target*, so switching a clip to PNG frames answers it
   * again); a clip already shorter than the length, which keeps its own; and a link, whose
   * `duration_secs` stays null until the backend announces one — a guess there would be a number
   * the row invented. It is computed for the link branch too, so a link that ever *does* arrive
   * with a duration says the same thing as every other clip rather than being a forgotten case.
   */
  const trimNote = trimmedLength(
    settings,
    info.duration_secs,
    outputIsTimeBased(catalog, info.category, info.is_animated, row.target),
  );

  const meta = (
    link !== null
      ? // A link has no size on disk and no probed duration until it has been fetched, so the site is
        // the whole of what is known about it before the run starts.
        [link.site_label, trimNote]
      : [
          info.format_name,
          formatBytes(info.size_bytes),
          info.duration_label ?? info.resolution_label,
          trimNote,
        ]
  )
    .filter((part): part is string => typeof part === "string" && part !== "")
    .join(" · ");

  /*
   * What the row is called, and what its tooltip says.
   *
   * `convert_link` announces `Started` with the output path before the download begins, and that
   * path's stem is the title yt-dlp resolved — so the row can stop showing a URL and start showing
   * "Rust in 100 Seconds" as soon as the first event lands, without a second round trip to ask. The
   * full URL stays as the tooltip: two videos from the same channel can share a title.
   */
  const title = link !== null && row.output !== null ? stemOf(row.output) : info.name;
  const tooltip = link !== null ? link.url : info.path;

  /*
   * What a screen reader is told this row *is*, when it stops on it: the file, and how it is
   * getting on.
   *
   * The name used to be the filename alone, so the one thing a stop could not tell you was the
   * thing the row exists to report — a user arrowing down twenty rows heard twenty filenames and
   * had to enter each one to find the failed one. A word is added for every state that is not
   * simply "waiting its turn", and a queued row keeps the bare filename it always had.
   *
   * Deliberately no percentage. The row is not a live region, but its accessible name is read
   * again whenever it changes under the ring, and FFmpeg reports progress several times a second —
   * which is the same "never finishes a sentence" failure the countdown was taken out of the
   * action bar's live region for (`ActionBar`, `aria-hidden` on the ETA). So the word changes once
   * per phase and no oftener: `Downloading` and `Converting` are the two halves of a link's job,
   * and the eye gets the number from `.row__status` beside it.
   */
  const spoken =
    !info.supported
      ? link !== null
        ? "unusable link"
        : "unsupported"
      : status === "running"
        ? link !== null && row.phase === "downloading"
          ? "downloading"
          : "converting"
        : status === "done"
          ? "converted"
          : status === "failed"
            ? "failed"
            : status === "skipped"
              ? "skipped"
              : "";

  /*
   * The helper this row is waiting on, if that is why it did not convert.
   *
   * "pdf needs Poppler installed (brew install …)" names a thing; this turns it into somewhere to
   * go. A *package*, because that is what the user would install: a microlink offering to install
   * `pdftohtml` names a program nobody ships on its own. Derived from the catalog, so it shows up on
   * exactly the rows a missing helper blocked — and disappears by itself once it is installed.
   */
  const missingHelper = missingHelperFor({ catalog, installPlans }, row);

  /*
   * The other kind of blocked row: nothing to install, one permission to grant.
   *
   * Safari keeps its cookies where only an app with Full Disk Access can read them, so a link asked
   * to borrow that sign-in fails with `Operation not permitted` however well-formed the request. The
   * message already offers the easier way out (pick another browser in Settings → Links); this is
   * the harder one, made one click instead of four levels of System Settings.
   */
  const fullDiskAccess = needsFullDiskAccess(row);

  /*
   * And the third kind: nothing to install, no permission to grant, a sign-in to sort out.
   *
   * The row used to end here — an accurate red sentence naming Settings → Links, and no verb. It
   * gets the affordance the two above already have, in the same place and the same shape, because
   * a failure with a remedy and no button is the bug this is fixing. Which verb depends on which
   * of the three sign-in endings it was: there is nothing to *fix* when nothing was ever set up.
   */
  const signIn = signInFailure(row);

  /*
   * How far along the line at the row's bottom edge is drawn.
   *
   * `rowProgress`, not `row.fraction`: a link's sample is about the half it is in, so a fetch at
   * 100% would draw a full line and then start the row again. The *text* still says the half's own
   * percentage, which is what `phaseStatus` is for.
   */
  const percent = Math.round(rowProgress(row) * 100);
  /*
   * The number a file row reads out: its one job's own percentage, which is the same thing.
   *
   * Except when there is no number to read. FFmpeg can only report a percentage against a duration,
   * and some files never say how long they are (a captured stream, a container written without a
   * header) — `Engine::run` reports `fraction: null` for the whole of such a job rather than the
   * `Some(0.0)` it used to invent. Printing `${row.fraction ?? 0}%` here put that invented zero back
   * on the screen: "0%" for a minute, next to a line pinned to nothing, which is what a hung job
   * looks like. `phaseStatus` is the sentence for it, the same one a link's indeterminate first
   * sample has always used, and [`indeterminate`] below is the line that goes with it.
   */
  const filePercent = Math.round((row.fraction ?? 0) * 100);
  /** No fraction, no bar: a travelling pulse, the same one an install draws while it works. */
  const indeterminate = row.fraction === null;
  // Capped so a 200-file queue does not spend five seconds fading itself in.
  const delay = `${Math.min(index, 9) * 26}ms`;

  return (
    <li
      className="row"
      data-status={status}
      data-supported={info.supported}
      data-selected={selected || undefined}
      style={{ animationDelay: delay }}
      // Selecting a row is what ⌫ acts on, so it must be reachable without a mouse.
      tabIndex={0}
      // A focusable list item takes no name from its own contents, so a screen reader landing here
      // used to announce "list item" and nothing else — with ⌫ next to it. The file's name is what
      // makes the stop worth having, and the state word beside it ([`spoken`]) is what saves
      // entering the row to find out how it went; everything else is read on the way through.
      aria-label={spoken === "" ? title : `${title}, ${spoken}`}
      aria-current={selected || undefined}
      onClick={() => select(id)}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return; // a select/button inside owns its own keys
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          select(id);
        }
      }}
    >
      <span className="row__kind" aria-hidden="true">
        {link !== null ? linkLabel(link.site) : extensionLabel(info.name)}
      </span>

      <div className="row__main">
        <span className="row__name" title={tooltip}>
          {title}
        </span>
        <span className="row__meta">
          {info.supported
            ? meta
            : (info.note ?? (link !== null ? "Unusable link" : "Unsupported file"))}
          {info.supported && info.note !== null && <span className="row__warn"> · {info.note}</span>}
        </span>
      </div>

      <div className="row__right">
        {status === "running" && (
          <span className="row__status">
            {link !== null || indeterminate
              ? phaseStatus(row.phase, row.fraction)
              : `${filePercent}%`}
            {etaSuffix(row.eta_secs)}
          </span>
        )}

        {status === "done" && row.output !== null && (
          <>
            <button
              type="button"
              className="row__output"
              title={row.output}
              onClick={() => void open(row.output ?? "")}
            >
              {baseName(row.output)}
              {row.bytes !== null ? ` · ${formatBytes(row.bytes)}` : ""}
            </button>
            <button
              type="button"
              className="microlink"
              onClick={() => void reveal(row.output ?? "")}
            >
              Reveal
            </button>
          </>
        )}

        {status === "failed" && (
          // A failure with no text would render an empty row with a Retry link and no reason.
          <span className="row__error" title={row.message ?? ""}>
            {row.message !== null && row.message !== "" ? row.message : "Conversion failed"}
          </span>
        )}

        {status === "skipped" && <span className="row__skipped">{row.message ?? "Skipped"}</span>}

        {(status === "failed" || status === "skipped") && missingHelper !== null && (
          // The whole point of the install button existing: one step from "it failed" to the thing
          // that fixes it, with the helper named rather than left as an exercise.
          <button
            type="button"
            className="microlink row__fix"
            title={`Open settings on ${missingHelper.name}`}
            onClick={(e) => {
              e.stopPropagation();
              showPackage(missingHelper.packageId);
            }}
          >
            Install {missingHelper.name}
          </button>
        )}

        {fullDiskAccess && (
          // Not an install and not a setting: the only fix is a permission, so the row offers the
          // trip rather than describing it. The ellipsis is macOS for "this opens something".
          <button
            type="button"
            className="microlink row__fix"
            title="Open System Settings → Privacy & Security → Full Disk Access"
            onClick={(e) => {
              e.stopPropagation();
              void openFullDiskAccess();
            }}
          >
            Open Full Disk Access…
          </button>
        )}

        {signIn !== null && (
          <button
            type="button"
            className="microlink row__fix"
            title="Show how to let this link use your sign-in"
            onClick={(e) => {
              e.stopPropagation();
              openSignInGuide();
            }}
          >
            {SIGN_IN_ACTION[signIn]}
          </button>
        )}

        {(status === "failed" || status === "skipped") && (
          // Skipped rows get this too: "Cancelled" and "a converted file already exists" are both
          // things the user fixes and retries, not dead ends.
          <button
            type="button"
            className="microlink"
            onClick={() => void retry(id)}
            disabled={running}
          >
            Retry
          </button>
        )}

        {/* Kept after the run, not only while queued: a clip that failed as MP4 may well succeed
            as WebM, and re-picking the format is the obvious fix. Locked for the whole duration of
            a batch this row belongs to - including once it has finished, because re-targeting a
            done row would silently discard a result the batch is still reporting on. */}
        {info.supported && info.category !== null && (
          <TargetPicker
            category={info.category}
            suggested={info.suggested_targets}
            value={row.target}
            disabled={running && inBatch}
            label={`Convert ${title} to`}
            onChange={(target) => setTarget(id, target)}
          />
        )}

        <button
          type="button"
          className="row__remove"
          aria-label={`Remove ${title}`}
          title="Remove"
          // Removing a row the batch still owns would hide it while Rust keeps converting it.
          disabled={owned}
          onClick={(e) => {
            e.stopPropagation();
            remove(id);
          }}
        >
          <span aria-hidden="true">×</span>
        </button>
      </div>

      {status === "running" &&
        (indeterminate ? (
          <span className="row__pulse" aria-hidden="true" />
        ) : (
          <span className="row__progress" style={{ width: `${percent}%` }} aria-hidden="true" />
        ))}
    </li>
  );
});

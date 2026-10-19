/**
 * Helper apps: what is already there, what is missing, what each missing one would unlock — and the
 * one button that installs it.
 *
 * One row per *thing a user installs*, not per binary this app spawns. Poppler is one Homebrew
 * formula behind `pdftoppm`, `pdftotext` and `pdftohtml`, so it is one row called "Poppler" with one
 * `brew install poppler` — three rows with the same command and a program name each was noise in a
 * list whose whole premise is restraint. The binaries stay in [`membersOf`], where they belong: they
 * answer "how much of it is here", never "what should I install".
 *
 * The list is deliberately lopsided. A helper that is present is one quiet line ("Found") and
 * nothing else, because a settings page full of solved problems reads as a chore list. One that is
 * missing — or only *partly* here — opens up: what it buys you in plain words, an Install button
 * when the backend says it can actually run one, and the exact command with a Copy affordance
 * underneath — the command is the fallback that always works, so it is shown whether or not the
 * button is.
 *
 * `brew install --cask libreoffice` downloads a few hundred megabytes, so the install streams its
 * output into a small monospace pane. Success is what Rust *observed* (it re-runs discovery), and
 * the failure message is surfaced verbatim: this section never invents an outcome.
 */
import { useEffect, useRef, useState } from "react";
import { copyText } from "../lib/clipboard";
import { memberCount, membersOf, missingMembers, presenceOf } from "../lib/packages";
import type { Presence } from "../lib/packages";
import type { PackageInstallPlan, ToolStatus } from "../lib/types";
import { useStore } from "../state/store";
import type { InstallRun } from "../state/store";

/**
 * Bootstrapping Homebrew is the user's job, not ours: it needs a password and rewrites `/opt`.
 * Shown, with a Copy, when `manager_available` is false — never run.
 *
 * Hardcoded because `PackageInstallPlan` carries the manager's *id*, not its home page or its own
 * install line. If the Rust side ever exposes those, this constant should come from there instead.
 */
const HOMEBREW_INSTALL =
  '/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"';

export function ToolsStrip(): React.JSX.Element | null {
  const tools = useStore((s) => s.catalog?.tools);
  const plans = useStore((s) => s.installPlans);
  const install = useStore((s) => s.install);
  /** An install the shell is running that has not named itself yet — see `State.foreignInstall`. */
  const foreignInstall = useStore((s) => s.foreignInstall);
  const highlighted = useStore((s) => s.highlightedPackage);
  const drawerOpen = useStore((s) => s.drawerOpen);
  const refresh = useStore((s) => s.refreshTools);
  const [busy, setBusy] = useState(false);
  const highlightRef = useRef<HTMLLIElement | null>(null);

  /*
   * Arriving from a row that failed for want of a helper: scroll that helper into view and put the
   * keyboard on its Install button, so the fix is one Return away. Deferred a frame because the
   * sheet moves focus to its own close button as it opens, and this has to happen after that.
   */
  useEffect(() => {
    if (!drawerOpen || highlighted === null) return undefined;
    const frame = requestAnimationFrame(() => {
      const node = highlightRef.current;
      if (node === null) return;
      node.scrollIntoView({ block: "nearest" });
      (node.querySelector<HTMLElement>(".toolbutton") ?? node).focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [drawerOpen, highlighted]);

  if (tools === undefined) return null;

  const check = (): void => {
    setBusy(true);
    void refresh().finally(() => setBusy(false));
  };

  const rows = helperRows(tools, plans);

  // One shared explanation rather than the same paragraph on every missing row: without Homebrew
  // nothing here has a button, and the reason is the same for all of them.
  const managerMissing = rows.some(
    (row) =>
      row.presence !== "installed" &&
      row.plan?.manager === "homebrew" &&
      row.plan.manager_available === false,
  );

  return (
    <section>
      <div className="tools__header">
        <h3 className="tools__title">Helper apps</h3>
        <button type="button" className="microlink" onClick={check} disabled={busy}>
          {busy ? "Checking…" : "Re-check"}
        </button>
      </div>

      {managerMissing && (
        <div className="tools__manager">
          <p className="tools__managertext">
            Homebrew is not installed, so Flint cannot install these for you. Install
            Homebrew from brew.sh first, then Re-check — or run a helper’s own command in Terminal.
          </p>
          <CommandLine command={HOMEBREW_INSTALL} label="the Homebrew install command" wrap />
        </div>
      )}

      <ul className="tools__list">
        {rows.map((row) => {
          const run = install !== null && install.packageId === row.id ? install : null;
          const highlight = highlighted === row.id;
          return (
            <HelperRow
              key={row.id}
              ref={highlight ? highlightRef : null}
              helper={row}
              run={run}
              highlight={highlight}
              /* One install at a time: the backend refuses a second, so the UI must not offer one.
                 That includes an install this window inherited and cannot name yet — it holds the
                 same slot, so no helper here has a button that could work. */
              otherInstallRunning={
                run === null && (foreignInstall || (install !== null && install.status === "running"))
              }
            />
          );
        })}
      </ul>

      {/* "Formats" was true of every helper until yt-dlp, which unlocks a *source* — a pasted link —
          rather than a format. The sentence names both, because a line that promised only formats
          next to a row whose own copy talks about links would be the sheet contradicting itself. */}
      <p className="tools__note">
        These are all optional: they only unlock the handful of formats FFmpeg cannot handle, and
        pasted links.
      </p>
    </section>
  );
}

/**
 * One line of the list: a package, or a program nothing installs (bundled FFmpeg, macOS `sips`).
 *
 * Flattened at the boundary so the row below renders one shape rather than branching on "is this a
 * package or a binary" five times. `id` is what `data-tool` carries and what an install is keyed by —
 * a package id where there is a package, and the program's own id where there is nothing to buy.
 */
interface Helper {
  id: string;
  /** What the user reads. A package's name, never a member binary's diagnostic label. */
  name: string;
  /** The plan, where one exists. Its absence *is* "there is nothing to install here". */
  plan: PackageInstallPlan | null;
  presence: Presence;
  /** How many of its programs are here, of how many it ships: "1 of 3". */
  found: number;
  total: number;
  bundled: boolean;
  /** Where it was found, for the state word's tooltip. */
  path: string | null;
  /** What to say when there is no command to run — `Tool::install_hint` in prose. */
  hint: string;
  /** Diagnostic only: the member programs this machine cannot find, for the tooltip. */
  absent: string[];
}

/**
 * The tool list and the plan list, joined into rows — one per package, one per unbuyable program,
 * in the order the tools arrive so the list does not reshuffle itself between two renders.
 *
 * A package appears at the position of its first member and swallows the rest, which is what turns
 * three Poppler binaries into one Poppler row. Presence comes from `lib/packages`, so "how much of
 * it is here" is derived in exactly one place: all members → installed, some → incomplete (and still
 * offered), none → absent.
 */
function helperRows(tools: ToolStatus[], plans: PackageInstallPlan[]): Helper[] {
  const rows: Helper[] = [];
  const seen = new Set<string>();
  for (const tool of tools) {
    const plan = plans.find((p) => p.tool_ids.includes(tool.id));
    if (plan === undefined) {
      rows.push({
        id: tool.id,
        name: tool.label,
        plan: null,
        presence: tool.available ? "installed" : "absent",
        found: tool.available ? 1 : 0,
        total: 1,
        bundled: tool.bundled,
        path: tool.path,
        hint: tool.install_hint,
        absent: [],
      });
      continue;
    }
    if (seen.has(plan.package_id)) continue;
    seen.add(plan.package_id);
    const members = membersOf(plan, tools);
    const { found, total } = memberCount(plan, tools);
    rows.push({
      id: plan.package_id,
      name: plan.name,
      plan,
      presence: presenceOf(plan, tools),
      found,
      total,
      bundled: false,
      path: members.find((member) => member.available)?.path ?? null,
      hint: members[0]?.install_hint ?? "",
      absent: missingMembers(plan, tools).map((member) => member.label),
    });
  }
  return rows;
}

/** "Included" / "Found" / "Incomplete" / "Missing" — the four words a row can end in. */
function stateWord(helper: Helper): string {
  if (helper.bundled) return helper.presence === "installed" ? "Included" : "Missing";
  if (helper.presence === "installed") return "Found";
  return helper.presence === "incomplete" ? "Incomplete" : "Missing";
}

/**
 * "1 of Poppler’s 3 programs is here. Installing it again should bring the rest."
 *
 * The awkward middle state needs words of its own: "Incomplete" alone says something is wrong
 * without saying what to do, and a row that stayed on "Missing" would be lying about a package the
 * user can see half of. The count is the honest part; *which* program is absent is a diagnostic and
 * lives in the tooltip, because nobody installs a program by name here.
 */
const partialLine = (helper: Helper): string =>
  `${helper.found} of ${helper.name}’s ${helper.total} programs ` +
  `${helper.found === 1 ? "is" : "are"} here. Installing it again should bring the rest.`;

interface RowProps {
  helper: Helper;
  run: InstallRun | null;
  highlight: boolean;
  otherInstallRunning: boolean;
  ref: React.Ref<HTMLLIElement> | null;
}

function HelperRow({
  helper,
  run,
  highlight,
  otherInstallRunning,
  ref,
}: RowProps): React.JSX.Element {
  const installPackage = useStore((s) => s.installPackage);
  const running = run !== null && run.status === "running";
  const plan = helper.plan;
  const here = helper.presence === "installed";
  // The detail block outlives "missing": a helper that just installed is still the one with a
  // verdict to show, and its success line must not vanish with the row that produced it.
  const open = !here || run !== null;

  return (
    <li
      ref={ref}
      className="tool"
      data-tool={helper.id}
      /* Not "is some of it here": a package the app can only half find cannot run the conversions
         that need the rest of it, so it is not available and it is still something to install. */
      data-available={here}
      data-missing={!here || undefined}
      data-presence={helper.presence}
      data-installable={plan !== null || undefined}
      data-highlight={highlight || undefined}
      /* Focusable only when it is the row the user was sent to, and only from script. */
      tabIndex={highlight ? -1 : undefined}
    >
      <div className="tool__head">
        <span className="tool__label">{helper.name}</span>
        <span className="tool__state" title={here ? (helper.path ?? "") : ""}>
          {stateWord(helper)}
        </span>
      </div>

      {open && (
        <div className="tool__detail">
          {helper.presence === "incomplete" && (
            <p className="tool__partial" title={`Not found: ${helper.absent.join(", ")}`}>
              {partialLine(helper)}
            </p>
          )}

          {!here && plan !== null && plan.unlocks.length > 0 && (
            <p className="tool__unlocks">
              {plan.unlocks.map((line: string) => (
                <span key={line} className="tool__unlock">
                  {line}
                </span>
              ))}
            </p>
          )}

          {!here && plan?.can_auto_install === true && (
            <>
              {plan.needs_admin && (
                <p className="tool__caution">
                  This one may ask for your Mac password. There is nowhere to type one here, so if it
                  does, the install has to be finished in Terminal.
                </p>
              )}
              <div className="tool__actions">
                <button
                  type="button"
                  className="toolbutton"
                  disabled={running || otherInstallRunning}
                  aria-label={`Install ${helper.name}`}
                  onClick={() => void installPackage(helper.id)}
                >
                  {running ? "Installing…" : "Install"}
                </button>
                {otherInstallRunning && (
                  <span className="tool__waiting">Another helper is installing</span>
                )}
              </div>
            </>
          )}

          {!here &&
            (plan !== null && plan.command !== "" ? (
              <CommandLine command={plan.command} label={`the ${helper.name} install command`} />
            ) : (
              // Nothing to run: FFmpeg ships inside the bundle, `sips` is part of macOS.
              <p className="tool__plain">{helper.hint}</p>
            ))}

          {run !== null && <InstallPanel run={run} label={helper.name} />}
        </div>
      )}

      {running && <span className="tool__pulse" aria-hidden="true" />}
    </li>
  );
}

/** The exact command, selectable, with a Copy that always works. */
function CommandLine({
  command,
  label,
  wrap = false,
}: {
  command: string;
  label: string;
  wrap?: boolean;
}): React.JSX.Element {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current);
    },
    [],
  );

  const copy = (): void => {
    void copyText(command).then((ok) => {
      if (!ok) return; // the code is `user-select: all`, so a click still selects the whole line
      setCopied(true);
      if (timer.current !== null) clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 1800);
    });
  };

  return (
    <div className="tool__command" data-wrap={wrap || undefined}>
      <code className="tool__hint">{command}</code>
      <button type="button" className="microlink" aria-label={`Copy ${label}`} onClick={copy}>
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

/**
 * Progress, honestly: a live status line plus the installer's own output.
 *
 * The log is the only evidence a four-minute download is alive, so it is open while the install
 * runs. Once it succeeds there is nothing left to read, so it collapses; a failure keeps it open,
 * because the last thing `brew` said is usually the answer.
 */
function InstallPanel({ run, label }: { run: InstallRun; label: string }): React.JSX.Element {
  const dismiss = useStore((s) => s.dismissInstall);
  const [showLog, setShowLog] = useState(true);
  const logRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    if (run.status === "ok") setShowLog(false);
    if (run.status === "running") setShowLog(true);
  }, [run.status]);

  useEffect(() => {
    const node = logRef.current;
    if (node !== null) node.scrollTop = node.scrollHeight;
  }, [run.lines.length, showLog]);

  const settled = run.status !== "running";

  return (
    <div className="install" data-status={run.status}>
      {/* One live region for the whole install: a screen reader hears "Installing…", then the
          verdict — not every one of `brew`'s hundred progress lines. */}
      <p className="install__state" role="status" aria-live="polite">
        {run.status === "running"
          ? `Installing ${label}… this can take a few minutes.`
          : (run.message ?? (run.status === "ok" ? `${label} is installed.` : "The install failed."))}
      </p>

      <div className="install__meta">
        {run.lines.length > 0 && (
          <button
            type="button"
            className="microlink"
            aria-expanded={showLog}
            onClick={() => setShowLog(!showLog)}
          >
            {showLog ? "Hide log" : "Show log"}
          </button>
        )}
        {settled && (
          <button type="button" className="microlink" onClick={dismiss}>
            Dismiss
          </button>
        )}
      </div>

      {showLog && run.lines.length > 0 && (
        // Focusable so the keyboard can scroll it. `role="log"` carries an implicit
        // `aria-live="polite"`, which would have a screen reader read every one of `brew`'s hundred
        // progress lines out loud over the status line above — the one thing that should be spoken.
        // Turning the announcements off keeps the role (this is a log, and it is navigable as one)
        // without the commentary.
        <pre
          ref={logRef}
          className="install__log"
          role="log"
          aria-live="off"
          tabIndex={0}
          aria-label={`Installer output for ${label}`}
        >
          {run.lines.join("\n")}
        </pre>
      )}
    </div>
  );
}

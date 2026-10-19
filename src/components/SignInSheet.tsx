/**
 * The walkthrough: five numbered steps, three buttons, and a way to find out whether you are fixed.
 *
 * This exists because one click is not always enough. Borrowing Chrome's sign-in is a button and
 * two seconds; borrowing Safari's costs a permission; a Mac with neither costs a browser extension
 * and an exported file. The old failure named Settings → Links and stopped there, which asked the
 * user to work all of that out from a red sentence — so the steps are written down, in order, in
 * the app's voice, with the button that performs each one next to the step it belongs to.
 *
 * The browser the walk is about is the one this Mac's evidence points at, and the sheet says why it
 * was picked before it asks anybody to act on it. Safari's steps are the three things that actually
 * block a user — grant, **relaunch**, check — plus the caveat that sinks a self-built copy, and the
 * easier browser, named only when this Mac really has one worth borrowing from.
 *
 * The last control is the point of the whole sheet. **Check sign-in** says what happened *here*, so
 * nobody has to start a conversion to discover whether the thing they just did helped — and for
 * Safari it answers from the permission itself, in no time at all.
 *
 * Built exactly like `LinksSheet`: one veil that swallows clicks, focus taken on open and handed
 * back on close, Esc out, ⌘↩ for the primary action, Tab trapped inside the card.
 */
import { useEffect, useRef } from "react";
import {
  browserReason,
  permissionFreeAlternative,
  preferredBrowser,
  useStore,
} from "../state/store";

const SELECTORS = 'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

/** One step: a sentence, and at most one thing the app can do about it from here. */
interface Step {
  text: string;
  action?: { label: string; title: string; run: () => void };
}

export function SignInSheet(): React.JSX.Element | null {
  const open = useStore((s) => s.signInOpen);
  const browsers = useStore((s) => s.cookieBrowsers);
  const settings = useStore((s) => s.settings);
  const check = useStore((s) => s.signInCheck);
  const close = useStore((s) => s.closeSignInGuide);
  const useBrowser = useStore((s) => s.useBrowserSignIn);
  const runCheck = useStore((s) => s.checkSignIn);
  const pickCookieFile = useStore((s) => s.pickCookieFile);
  const openFullDiskAccess = useStore((s) => s.openFullDiskAccess);

  const cardRef = useRef<HTMLDivElement>(null);
  const primaryRef = useRef<HTMLButtonElement>(null);
  /** Whatever had focus when the sheet appeared, so closing hands it straight back. */
  const opener = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return undefined;
    const active = document.activeElement;
    // `<body>` is not an opener: the sheet can arrive from a question that has just closed, and
    // handing the ring back to the body would start the next Tab at the top of the window.
    opener.current = active instanceof HTMLElement && active !== document.body ? active : null;
    primaryRef.current?.focus();
    return () => {
      const previous = opener.current;
      opener.current = null;
      if (previous !== null && previous.isConnected) {
        previous.focus();
        return;
      }
      document.querySelector<HTMLElement>(".dropzone")?.focus();
    };
  }, [open]);

  if (!open) return null;

  const offer = preferredBrowser(browsers);
  const label = offer?.label ?? "your browser";
  const checking = check?.checking === true;
  const link = settings?.link ?? null;
  /**
   * The one clause behind the name at the top of the walk.
   *
   * Said here as well as in the question because the sheet is reachable from Settings, where no
   * question was ever asked — and a walk that opens "Sign in to the site in Safari" without saying
   * why Safari is a walk that has to be taken on trust.
   */
  const why = offer === null ? null : `${offer.label} — ${browserReason(offer)}.`;
  /*
   * A browser worth switching to rather than granting a permission for, or null.
   *
   * Only ever one that is installed *and* has a sign-in store with something in it: telling a user
   * weighing up four levels of System Settings that Chrome is easier, when their Chrome has never
   * been signed in to anything, is the same wrong answer that started this pass with a friendlier
   * face on it.
   */
  const easier = permissionFreeAlternative(browsers, offer);

  /**
   * What the sheet says is configured right now, in the words the user chose it with.
   *
   * Shown because every step below changes it and the verdict alone cannot say what was tested: a
   * user who reads "that sign-in works" is owed the name of the sign-in that works.
   */
  const chosen = link === null ? "" : link.cookie_browser.trim();
  // The label, never the id: `chrome` is what yt-dlp is handed and "Chrome" is what a person reads.
  // A name the allowlist does not know is shown as it was written, because that is the thing that
  // needs correcting and hiding it would leave the sentence describing a browser nobody chose.
  const chosenLabel =
    browsers.find((browser) => browser.id === chosen.toLowerCase())?.label ?? chosen;
  const source =
    link === null || link.cookies === "none"
      ? "No sign-in configured yet."
      : link.cookies === "browser"
        ? chosen === ""
          ? "A browser is chosen as the source, but not which browser."
          : `Reading the sign-in from ${chosenLabel}.`
        : link.cookie_file === null || link.cookie_file.trim() === ""
          ? "A cookies.txt file is the source, but no file is chosen."
          : `Reading the sign-in from ${link.cookie_file}.`;

  /*
   * The steps, in the order a person does them, and only the ones this Mac can act on.
   *
   * Step two is the whole flow condensed: it saves the source and checks it in one press, because
   * choosing a browser and then being left to prove it yourself is the dead end this sheet
   * replaces. On a machine with no allowlisted browser it is not offered at all — an offer to
   * borrow from software the user does not have is the same lie in a friendlier font.
   */
  const steps: Step[] = [];
  if (offer === null) {
    steps.push({
      text:
        "Flint found none of the browsers it can borrow a sign-in from, so the route " +
        "here is a file you export yourself. Everything below is about that.",
    });
  } else {
    steps.push({
      text: `Sign in to the site in ${label} first, the way you normally would. Flint never sees your password.`,
    });
    steps.push({
      text: `Press the button and the app borrows that sign-in from ${label}. It reads the cookies and nothing else.`,
      action: {
        label: `Use ${label}`,
        title: `Read the sign-in from ${label} and check it`,
        run: () => void useBrowser(offer.id),
      },
    });
    // Chrome, Edge, Brave and the rest keep their cookie key in the login Keychain and macOS asks
    // before handing it over. Safari's jar is not encrypted that way — it is behind Full Disk
    // Access instead — so telling a Safari user to expect a Keychain prompt would send them
    // looking for a dialog that never comes.
    if (offer.id !== "safari") {
      steps.push({
        text: navigator.userAgent.includes("Windows")
          ? "Windows browser encryption can prevent cookie access. If checking fails, use Firefox or choose an exported cookies.txt file below."
          :
          "macOS may ask to unlock the browser’s keychain the first time. Choose Allow — refuse it " +
          "and the cookies come back encrypted, which the site sees as no sign-in at all.",
      });
    }
  }
  /*
   * Safari's three steps, and only on a Mac that has Safari.
   *
   * Elsewhere this is a step about software the user does not own, and a permission they would be
   * granting for nothing — the same dead option the browser menu stopped offering.
   *
   * The wording is the measured one, and each sentence is here because it is a thing that stops
   * somebody who has done everything they were told: macOS only consults the grant when a process
   * *starts*, so an app that was already open when the switch was flipped stays denied; and the
   * grant is keyed to a code signature, so a copy built at home is a different app after every
   * rebuild while its old entry sits in that list looking switched on.
   */
  if (browsers.some((browser) => browser.id === "safari" && browser.installed)) {
    steps.push({
      text:
        "Safari is the one browser whose cookies sit behind Full Disk Access. Grant it in the list " +
        "that opens, then quit Flint and open it again — the permission only reaches " +
        "an app that was started after it was given. Then press Check sign-in below.",
      action: {
        label: "Open Full Disk Access…",
        title: "Open System Settings → Privacy & Security → Full Disk Access",
        run: () => void openFullDiskAccess(),
      },
    });
    steps.push({
      text:
        "If you built this copy of Flint yourself, it loses that grant every time it " +
        "is rebuilt, and the old entry stays in the list looking switched on. Switch it off and on " +
        "again there.",
    });
    // Named only where it is real. "Use another browser instead" is the cheapest advice in this
    // sheet and the easiest to make useless: on the Mac this pass came from, the other browser was
    // a Chrome with an empty cookie store, and sending that user to it would have cost them the
    // permission *and* the sign-in.
    if (easier !== null) {
      steps.push({
        text: `Or borrow ${easier.label} instead: it is here too, it has a sign-in saved, and it needs no permission at all.`,
      });
    }
  }
  steps.push({
    text:
      "Or export a cookies.txt from your browser and choose it here. Be warned: browsers cannot do " +
      "that on their own, so it takes a cookies.txt extension.",
    action: {
      label: "Choose cookies.txt…",
      title: "Choose an exported cookies.txt file",
      run: () => void pickCookieFile(),
    },
  });

  /**
   * Esc closes, ⌘↩ checks, Tab cycles inside the card.
   *
   * ⌘↩ rather than ↩ for the same reason the paste box uses it: the sheet is full of buttons, and
   * a bare Return on one of them is that button's own activation, which is right.
   */
  const onKeyDown = (event: React.KeyboardEvent): void => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      if (!checking) void runCheck();
      return;
    }
    if (event.key !== "Tab" || cardRef.current === null) return;
    const focusable = Array.from(cardRef.current.querySelectorAll<HTMLElement>(SELECTORS)).filter(
      (el) => !el.hasAttribute("disabled") && el.tabIndex !== -1,
    );
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (first === undefined || last === undefined) return;
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === cardRef.current)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div className="promptveil" onClick={close}>
      <div
        ref={cardRef}
        className="signin"
        role="dialog"
        aria-modal="true"
        aria-labelledby="signin-title"
        aria-describedby="signin-note"
        // Focusable from script only: the card itself is where Esc and the trap are heard.
        tabIndex={-1}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <h2 className="signin__title" id="signin-title">
          Let a link use your sign-in
        </h2>

        <p className="signin__note" id="signin-note">
          Some videos are only handed over to someone who is signed in. Flint can borrow
          the sign-in a browser already has — it never asks you for a password.
        </p>

        {/* Why this browser and not one of the others. A size and a modification time is the whole
            of the evidence, and the clause is written so it never suggests otherwise. */}
        {why !== null && <p className="signin__why">{why}</p>}

        <ol className="signin__steps">
          {steps.map((step) => (
            <li key={step.text} className="signin__step">
              {step.text}
              {step.action !== undefined && (
                <span className="signin__do">
                  <button
                    type="button"
                    className="button"
                    title={step.action.title}
                    disabled={checking}
                    onClick={step.action.run}
                  >
                    {step.action.label}
                  </button>
                </span>
              )}
            </li>
          ))}
        </ol>

        <p className="signin__source">{source}</p>

        {/* The verdict, in place. `role="status"` rather than `alert`: this is the answer to a
            question the user asked by pressing a button, not an interruption — and "unreadable"
            and "refused" are different problems, which is what the colour is keyed off. */}
        {check !== null && (
          <p className="signin__verdict" data-result={check.result ?? "pending"} role="status">
            {check.message}
          </p>
        )}

        <div className="signin__actions">
          <button type="button" className="microlink" onClick={close}>
            Done
          </button>
          <button
            ref={primaryRef}
            type="button"
            className="promptbutton"
            disabled={checking}
            onClick={() => void runCheck()}
          >
            {checking ? "Checking…" : "Check sign-in"}
          </button>
        </div>
      </div>
    </div>
  );
}

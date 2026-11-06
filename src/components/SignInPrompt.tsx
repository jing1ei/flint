/**
 * "2 links need a sign-in. Use your Chrome sign-in and try again?"
 *
 * The install question's twin (`InstallPrompt`), down to the card, the veil, the one brass action
 * and the single fade — because it is the same moment: a batch has just come back with nothing to
 * show, and the app has exactly one thing to say about why.
 *
 * What differs is what Yes does. The helper question opens Settings and stops, because installing
 * costs a download and a password. This one *finishes the job*: it saves the sign-in source, checks
 * it against a public video, and retries the rows the wall stopped. So its button carries no
 * ellipsis for the browser case — nothing further is being opened — and it does carry one for the
 * other two, where the honest answer is a trip to System Settings or to the guided sheet.
 *
 * The browser it names is the one this Mac's own evidence points at (`list_cookie_browsers`, ranked
 * in Rust). It never offers software the user does not own, it never offers a cookie store that is
 * empty or missing, and it never promises one click where Safari's Full Disk Access or an exported
 * cookies.txt is the real price. It also says, in one clause, *why* that browser and not another —
 * because a name with no reason behind it is what the old rule gave, and the old rule was wrong.
 */
import { useEffect, useRef } from "react";
import type { SignInRemedy } from "../state/store";
import { signInConfirmLabel, signInQuestion, useStore } from "../state/store";

const SELECTORS = 'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

/**
 * The line under the question: what Yes is about to do, before it is clicked.
 *
 * Every one of them promises only what that path can deliver. "Nothing is downloaded and no
 * password is asked for" is the sentence a user needs before they let an app near a browser jar,
 * and it is true: yt-dlp reads the cookie database, and this app never sees the account.
 *
 * The two that name a browser open with the reason it was named — "Safari — you used it recently" —
 * which is the whole of what the app knows and, as importantly, the whole of what it claims: a
 * file's size and the minute it was last written, never a glance inside it.
 */
const promise = (remedy: SignInRemedy): string => {
  switch (remedy.kind) {
    case "browser":
      return (
        `${remedy.label} — ${remedy.reason}. Flint borrows the sign-in it already ` +
        "has, checks it against a public video, and runs those links again if it works. Nothing " +
        "is downloaded and no password is asked for."
      );
    case "full_disk_access":
      return (
        `${remedy.label} — ${remedy.reason}. System Settings opens on Privacy & Security → Full ` +
        "Disk Access; grant it there, then quit Flint and open it again, because the " +
        "permission only reaches an app that was started after it was given. The walkthrough has " +
        "the rest, including the easier route if this Mac has one."
      );
    case "cookie_file":
      return (
        "The next step is a cookies.txt exported from a browser, which needs a browser extension. " +
        "The walkthrough shows what to do and can check the file once you have chosen it."
      );
    case "guide":
      return (
        "That sign-in is the one already configured, so borrowing it again would fail the same " +
        "way. The walkthrough covers signing in afresh, the keychain prompt and the cookies.txt " +
        "route — and can check whichever you try, without running a conversion."
      );
  }
};

export function SignInPrompt(): React.JSX.Element | null {
  const prompt = useStore((s) => s.signInPrompt);
  const confirm = useStore((s) => s.confirmSignInPrompt);
  const dismiss = useStore((s) => s.dismissSignInPrompt);
  const guide = useStore((s) => s.openSignInGuide);
  const cardRef = useRef<HTMLDivElement>(null);
  const primaryRef = useRef<HTMLButtonElement>(null);
  /** Whatever had focus when the card appeared, so dismissing hands it straight back. */
  const opener = useRef<HTMLElement | null>(null);

  const open = prompt !== null;

  useEffect(() => {
    if (!open) return undefined;
    const active = document.activeElement;
    opener.current = active instanceof HTMLElement && active !== document.body ? active : null;
    primaryRef.current?.focus();
    return () => {
      const previous = opener.current;
      opener.current = null;
      /*
       * The same restore either way, and that is the point of it.
       *
       * A hand-over to the walkthrough used to leave the ring alone so as not to fight the sheet
       * for it — but this card's own button is being detached in this very commit, so "alone"
       * meant `<body>`, and the sheet, which reads `document.activeElement` when it opens, then
       * had nothing to hand back to. Closing the walkthrough dropped the keyboard on the canvas,
       * or on nothing at all when a queue was standing where the canvas would be. Putting the ring
       * back on the control that was live before the question appeared happens in the same passive
       * flush, before the sheet takes focus, so the sheet inherits a real opener and Done returns
       * the user to exactly where Esc on this card would have.
       */
      if (previous !== null && previous.isConnected) previous.focus();
    };
  }, [open]);

  if (prompt === null) return null;
  const remedy = prompt.remedy;

  const yes = (): void => {
    void confirm();
  };

  /** The longer way round, for a user who wants to know what they are agreeing to first. */
  const walkThrough = (): void => {
    guide();
  };

  /**
   * Enter confirms, Esc declines, Tab cycles inside the card — the install question's keys exactly,
   * because two cards that look identical and answer the keyboard differently would be worse than
   * either of them alone.
   */
  const onKeyDown = (event: React.KeyboardEvent): void => {
    if (event.key === "Escape") {
      event.preventDefault();
      dismiss();
      return;
    }
    if (event.key === "Enter" && !(event.target instanceof HTMLButtonElement)) {
      event.preventDefault();
      yes();
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
    <div className="promptveil" onClick={dismiss}>
      <div
        ref={cardRef}
        className="prompt"
        role="dialog"
        aria-modal="true"
        aria-labelledby="signin-prompt-title"
        aria-describedby="signin-prompt-body"
        // Focusable from script only: the card itself is where Esc and the trap are heard.
        tabIndex={-1}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <h2 className="prompt__title" id="signin-prompt-title">
          {signInQuestion(prompt)}
        </h2>

        <div className="prompt__body" id="signin-prompt-body">
          <p className="prompt__note">{promise(remedy)}</p>
        </div>

        <div className="prompt__actions">
          <button type="button" className="microlink" onClick={dismiss}>
            Not now
          </button>
          {/* The middle way, and only where Yes is already the whole answer: with Safari or a
              cookies.txt the primary button opens the walkthrough itself, and two buttons onto the
              same sheet would be one of them lying about being different. */}
          {remedy.kind === "browser" && (
            <button type="button" className="microlink" onClick={walkThrough}>
              Walk me through it…
            </button>
          )}
          <button ref={primaryRef} type="button" className="promptbutton" onClick={yes}>
            {signInConfirmLabel(remedy)}
          </button>
        </div>
      </div>
    </div>
  );
}

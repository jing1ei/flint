/**
 * "Three files couldn't be converted. PDF needs LibreOffice. Shall I take you there?"
 *
 * The one modal in the app, and it earns its interruption by being the only thing that *tells* the
 * user why a batch came back empty: the failed row already carries a microlink to the helper, but a
 * 10px link at the end of a red row is easy to look straight past. So the app says it once, quietly
 * — a small card on the dimmed window, a hairline, one brass action, one fade.
 *
 * It never starts an install. Yes is a journey, not a purchase: it opens Settings on that helper
 * with the keyboard already on its Install button, and the user pulls the trigger there. The
 * primary action is therefore titled with an ellipsis, which is macOS for "this opens something,
 * it does not do it".
 */
import { useEffect, useRef } from "react";
import { pluralFiles } from "../lib/format";
import { useStore } from "../state/store";
import type { BlockedHelper } from "../state/store";

/**
 * `PDF needs LibreOffice` / `PDF and Word (docx) need LibreOffice` / `PDF and 2 more formats …`
 *
 * The name is the *package's* — "PDF needs Poppler", never "PDF needs pdftohtml". A card asking for
 * a binary would be asking for something the user cannot buy, and the one thing this card must do is
 * be actionable.
 */
function stake(helper: BlockedHelper): string {
  const [first, second, ...rest] = helper.formats;
  if (first === undefined) return `${helper.name} is missing`;
  if (second === undefined) return `${first} needs ${helper.name}`;
  const subject =
    rest.length === 0
      ? `${first} and ${second}`
      : `${first} and ${rest.length + 1} more formats`;
  return `${subject} need ${helper.name}`;
}

const SELECTORS = 'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

export function InstallPrompt(): React.JSX.Element | null {
  const prompt = useStore((s) => s.installPrompt);
  const confirm = useStore((s) => s.confirmInstallPrompt);
  const dismiss = useStore((s) => s.dismissInstallPrompt);
  const cardRef = useRef<HTMLDivElement>(null);
  const primaryRef = useRef<HTMLButtonElement>(null);
  /** Whatever had focus when the card appeared, so dismissing hands it straight back. */
  const opener = useRef<HTMLElement | null>(null);
  /** Yes gives the keyboard to Settings; restoring focus on the way out would fight it for it. */
  const handingOver = useRef(false);

  const open = prompt !== null;

  useEffect(() => {
    if (!open) return undefined;
    const active = document.activeElement;
    opener.current = active instanceof HTMLElement ? active : null;
    primaryRef.current?.focus();
    return () => {
      const previous = opener.current;
      opener.current = null;
      if (handingOver.current) {
        handingOver.current = false;
        return;
      }
      if (previous !== null && previous.isConnected) previous.focus();
    };
  }, [open]);

  if (prompt === null) return null;
  const primary = prompt.tools[0];
  if (primary === undefined) return null;

  const yes = (): void => {
    handingOver.current = true;
    confirm();
  };

  /**
   * Enter confirms, Esc dismisses, Tab cycles inside the card. The same trap as the settings sheet:
   * the backdrop swallows every click, so the keyboard must not be able to reach the queue either.
   * Enter is only handled away from the buttons — on one, the browser's own activation is right.
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

  const many = prompt.tools.length > 1;

  return (
    <div className="promptveil" onClick={dismiss}>
      <div
        ref={cardRef}
        className="prompt"
        role="dialog"
        aria-modal="true"
        aria-labelledby="prompt-title"
        aria-describedby="prompt-body"
        // Focusable from script only: the card itself is where Esc and the trap are heard.
        tabIndex={-1}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <h2 className="prompt__title" id="prompt-title">
          {prompt.files} {pluralFiles(prompt.files)} couldn’t be converted
        </h2>

        <div className="prompt__body" id="prompt-body">
          {many ? (
            <ul className="prompt__list">
              {prompt.tools.map((helper) => (
                <li key={helper.packageId} className="prompt__item">
                  <span className="prompt__stake">{stake(helper)}</span>
                  <span className="prompt__count">
                    {helper.files} {pluralFiles(helper.files)}
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="prompt__line">{stake(primary)}.</p>
          )}

          {/* Said before the click, because the button's title alone cannot promise it: this
              opens Settings and stops there. The nbsp keeps the verb next to the button it names,
              which is where the line would otherwise break. */}
          <p className="prompt__note">
            Settings opens on {primary.name}. Nothing is installed until you
            click{"\u00a0"}Install there.
          </p>
        </div>

        <div className="prompt__actions">
          <button type="button" className="microlink" onClick={dismiss}>
            Not now
          </button>
          <button ref={primaryRef} type="button" className="promptbutton" onClick={yes}>
            Install {primary.name}…
          </button>
        </div>
      </div>
    </div>
  );
}

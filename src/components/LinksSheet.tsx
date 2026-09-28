/**
 * Paste individual video and music links into the conversion queue.
 *
 * Open from the toolbar, File menu, or ⌘L; ⌘V anywhere opens it already filled in.
 *
 * The same idiom as the install question (`InstallPrompt`): a veil that swallows clicks, one card,
 * Esc out, Tab trapped inside, and focus handed back to whatever opened it. What is different is
 * that this one *judges as you type*: every line is sent to `inspect_links` behind a 200ms debounce,
 * and the lines that came back refused are listed underneath with the backend's own sentence
 * against them. Those sentences already say what to do instead ("that is a playlist, open the videos
 * you want"), so they are shown verbatim rather than summarised into "3 invalid links".
 *
 * With yt-dlp absent the box still adds links. Refusing would be a second opinion about a fact the
 * row can state for itself: it fails with the install hint, which is honest and one click from fixed.
 */
import { useEffect, useRef, useState } from "react";
import type { LinkInspection, LinkRow } from "../lib/types";
import { useStore } from "../state/store";

const SELECTORS = 'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

/** How long the box waits before asking the backend what it is holding. */
const DEBOUNCE_MS = 200;

export function LinksSheet(): React.JSX.Element | null {
  const open = useStore((s) => s.linksOpen);
  const prefill = useStore((s) => s.linksPrefill);
  const support = useStore((s) => s.linkSupport);
  const refusal = useStore((s) => s.linksError);
  const close = useStore((s) => s.closeLinks);
  const check = useStore((s) => s.checkLinks);
  const clearError = useStore((s) => s.clearLinksError);
  const add = useStore((s) => s.addLinks);
  const showPackage = useStore((s) => s.showPackage);

  const [text, setText] = useState("");
  const [inspection, setInspection] = useState<LinkInspection | null>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  const boxRef = useRef<HTMLTextAreaElement>(null);
  /** Whatever had focus when the box appeared, so closing hands it straight back. */
  const opener = useRef<HTMLElement | null>(null);
  /** The trip into Settings takes the keyboard with it; restoring focus would fight it for it. */
  const handingOver = useRef(false);

  useEffect(() => {
    if (!open) return undefined;
    const active = document.activeElement;
    // `<body>` is not an opener. ⌘V is the way in, and it needs no click first, so on a window
    // nobody has touched this really is where the keyboard stands — and handing it back afterwards
    // left the ring on `<body>`, where the next Tab starts again at the top of the window.
    opener.current = active instanceof HTMLElement && active !== document.body ? active : null;
    boxRef.current?.focus();
    return () => {
      const previous = opener.current;
      opener.current = null;
      if (handingOver.current) {
        handingOver.current = false;
        return;
      }
      if (previous !== null && previous.isConnected && !previous.closest('[inert]')) {
        previous.focus({ preventScroll: true });
        return;
      }
      // Adding the first links removes the empty canvas that opened this dialog.
      const fallback = document.querySelector<HTMLElement>(".dropzone")
        ?? document.querySelector<HTMLElement>(".row")
        ?? document.querySelector<HTMLElement>('.titlebar__button[aria-label="Paste links"]');
      fallback?.focus({ preventScroll: true });
    };
  }, [open]);

  // Each opening starts from what the clipboard offered (or from nothing), never from the text of
  // the last one: a box that came back holding a link the user has already queued reads as a bug.
  useEffect(() => {
    if (!open) return;
    setText(prefill);
    setInspection(null);
  }, [open, prefill]);

  /*
   * Judgement, debounced.
   *
   * `inspect_links` is pure and instant in Rust, but it is still an IPC round trip per keystroke, and
   * the answer for a half-typed URL is a refusal nobody needs to read. The timer is cleared on every
   * edit and on the way out, and a late answer for text that has since changed is dropped: without
   * that, backspacing over a line left its refusal on screen.
   */
  useEffect(() => {
    if (!open) return undefined;
    setInspection(null);
    clearError();
    if (text.trim() === "") {
      // An empty box is never sent to the backend, so nothing else would ever clear a refusal it
      // gave about text that has since been deleted: "You pasted 21 links" beside an empty box.
      clearError();
      return undefined;
    }
    let live = true;
    const timer = setTimeout(() => {
      void check(text.split("\n")).then((result) => {
        if (live) setInspection(result);
      });
    }, DEBOUNCE_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [open, text, check, clearError]);

  if (!open) return null;

  const cap = support?.max_links ?? inspection?.limit ?? null;
  const accepted: LinkRow[] = (inspection?.links ?? []).filter((link) => link.supported);
  const refused: LinkRow[] = (inspection?.links ?? []).filter((link) => !link.supported);

  const submit = (): void => {
    if (accepted.length === 0) return;
    add(accepted);
  };

  /** Settings, on the package that makes links work. It installs nothing — that click is theirs. */
  const toSettings = (): void => {
    const packageId = support?.package_id;
    if (packageId === undefined) return;
    handingOver.current = true;
    close();
    showPackage(packageId);
  };

  /**
   * Esc closes, ⌘↩ adds, Tab cycles inside the card.
   *
   * ⌘↩ rather than ↩: the box is a textarea, and Return in it is a new line — which is how a second
   * link gets pasted. Same trap as the install question: the veil swallows every click, so the
   * keyboard must not be able to reach the queue behind it either.
   */
  const onKeyDown = (event: React.KeyboardEvent): void => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      submit();
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
        className="links"
        role="dialog"
        aria-modal="true"
        aria-labelledby="links-title"
        aria-describedby="links-note"
        // Focusable from script only: the card itself is where Esc and the trap are heard.
        tabIndex={-1}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <h2 className="links__title" id="links-title">
          Paste links
        </h2>

        <p className="links__note" id="links-note">
          {cap === null
            ? "YouTube, Bilibili, QQ Music, NetEase Music, SoundCloud and Bandcamp. One track or video per line."
            : `YouTube, Bilibili, QQ Music, NetEase Music, SoundCloud and Bandcamp: one per line, up to ${cap} at a time.`}
        </p>

        <textarea
          ref={boxRef}
          className="links__box"
          aria-label="Media links, one per line"
          spellCheck={false}
          rows={4}
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            setInspection(null);
            clearError();
          }}
        />

        {/* The count and the whole-paste refusal share one line: "you pasted 24 links" is the
            answer to "how many is this", and both at once would say the same thing twice. */}
        {refusal !== null ? (
          <p className="links__error" role="alert">
            {refusal}
          </p>
        ) : (
          inspection !== null && (
            <p className="links__count">
              {cap === null
                ? `${accepted.length} ${accepted.length === 1 ? "link" : "links"}`
                : `${accepted.length} of ${cap} links`}
            </p>
          )
        )}

        {refused.length > 0 && (
          <ul className="links__refusals">
            {refused.map((link) => (
              <li key={link.id} className="links__refusal">
                <span className="links__refusalline" title={link.url}>
                  {link.url}
                </span>
                {/* The backend's own sentence. It already names the next step; rewriting it here
                    would be a second, shorter, less useful copy of the same advice. */}
                <span className="links__refusalnote">{link.note}</span>
              </li>
            ))}
          </ul>
        )}

        {support !== null && !support.tool_installed && (
          <p className="links__helper">
            These need yt-dlp, which is not installed.{" "}
            <button type="button" className="microlink" onClick={toSettings}>
              Install yt-dlp…
            </button>
          </p>
        )}

        {support !== null && (
          <p className="links__where" title={support.destination}>
            Files land in {support.destination}
          </p>
        )}

        <div className="links__actions">
          <button type="button" className="microlink" onClick={close}>
            Cancel
          </button>
          <button
            type="button"
            className="promptbutton"
            disabled={accepted.length === 0}
            onClick={submit}
          >
            {accepted.length > 1 ? `Add ${accepted.length} links` : "Add link"}
          </button>
        </div>
      </div>
    </div>
  );
}

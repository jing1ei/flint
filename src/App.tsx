/**
 * One window, no routing.
 *
 * A persistent toolbar sits above the drop surface or queue. Conversion actions appear once
 * files exist; settings and source dialogs share the same window.
 */
import { useEffect } from "react";
import { ActionBar } from "./components/ActionBar";
import { DropZone } from "./components/DropZone";
import { FileList } from "./components/FileList";
import { InstallPrompt } from "./components/InstallPrompt";
import { LinksSheet } from "./components/LinksSheet";
import { SignInPrompt } from "./components/SignInPrompt";
import { SignInSheet } from "./components/SignInSheet";
import { SettingsDrawer } from "./components/SettingsDrawer";
import { TitleBar } from "./components/TitleBar";
import { isTauriRuntime } from "./lib/ipc";
import { useStore } from "./state/store";

/** True when the keystroke happened inside something that owns its own keyboard handling. */
function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable;
}

export function App(): React.JSX.Element {
  const init = useStore((s) => s.init);
  const hasFiles = useStore((s) => s.order.length > 0);
  const dragging = useStore((s) => s.dragging);
  const error = useStore((s) => s.error);
  const clearError = useStore((s) => s.clearError);
  const initializationFailed = useStore((s) => s.initializationFailed);
  const notice = useStore((s) => s.notice);
  const clearNotice = useStore((s) => s.clearNotice);
  /*
   * A batch inherited from the page load this one replaced has no rows here at all (see
   * `State.inheritedBatch`), and an empty window with a conversion running in it still has to say
   * so and still has to offer Stop. That is the one state in which the empty canvas is not silent.
   */
  const converting = useStore((s) => s.phase === "running");

  useEffect(() => {
    void init();
  }, [init]);

  /*
   * Only the two keys the menu bar does not own: Esc dismisses whatever is over the window (the
   * question first, then the paste box, then the sheet), and ⌫ removes the selected row.
   *
   * ⌘, ⌘O ⇧⌘O ⌘L ⌘↩ ⌘. ⇧⌘⌫ are menu accelerators. macOS consumes them at the menu and Rust emits
   * `menu://action`, so binding them here as well would fire every command twice. Outside Tauri
   * there is no menu, so the ⌘ set is registered — and `defaultPrevented` keeps it and the mock
   * backend's own bridge from both acting on the same keystroke.
   */
  useEffect(() => {
    const native = isTauriRuntime();
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented) return;
      const store = useStore.getState();
      if (store.cropOpen) return;
      const meta = event.metaKey || event.ctrlKey;

      if (event.key === "Escape" && store.installPrompt !== null) {
        // Before the sheet, and here rather than only on the card: the card hears Esc while the
        // keyboard is inside it, and focus can be outside it — the element that had focus when the
        // question was raised may have unmounted with the row that produced it.
        event.preventDefault();
        store.dismissInstallPrompt();
        return;
      }
      if (event.key === "Escape" && store.signInPrompt !== null) {
        // The sign-in question is the install question's twin and is dismissed by the same key, in
        // the same place and for the same reason: the row whose failure raised it may be gone.
        event.preventDefault();
        store.dismissSignInPrompt();
        return;
      }
      if (event.key === "Escape" && store.signInOpen) {
        // Then the walkthrough, which — like the paste box — replaces the settings sheet rather
        // than covering it, so it is never underneath anything but the two questions above.
        event.preventDefault();
        store.closeSignInGuide();
        return;
      }
      if (event.key === "Escape" && store.linksOpen) {
        // Between the two, in the order they can cover each other: the install question refuses to
        // open the paste box at all, and the paste box replaces the settings sheet rather than
        // stacking over it, so at most one of the three is ever the outermost thing.
        event.preventDefault();
        store.closeLinks();
        return;
      }
      if (event.key === "Escape" && store.drawerOpen) {
        event.preventDefault();
        store.setDrawer(false);
        return;
      }
      // The sheet sits over a scrim that swallows every click, so while it is open the keyboard
      // must not reach the queue either: plain ⌫ used to delete the row selected behind the sheet.
      // The install prompt and the paste box are modals over the same window and count for the same
      // reason. The ⌘ set below stays live, because the native menu bar stays live in that state too.
      if (
        (event.key === "Backspace" || event.key === "Delete") &&
        !meta &&
        !store.drawerOpen &&
        !store.linksOpen &&
        !store.signInOpen &&
        store.installPrompt === null &&
        store.signInPrompt === null &&
        !isTyping(event.target)
      ) {
        if (store.selectedId === null) return;
        event.preventDefault();
        // `removeFile` itself refuses rows the running batch owns, so both paths agree.
        store.removeFile(store.selectedId);
        return;
      }
      if (native || !meta) return;

      const key = event.key.toLowerCase();
      if (key === "," || (event.shiftKey && event.code === "Comma")) {
        event.preventDefault();
        if (event.shiftKey) store.openSkin();
        else store.setDrawer(!store.drawerOpen);
      } else if (key === "o") {
        event.preventDefault();
        void (event.shiftKey ? store.openFolder() : store.openFiles());
      } else if (key === "l") {
        event.preventDefault();
        store.openLinks();
      } else if (key === "enter") {
        event.preventDefault();
        void store.start();
      } else if (key === ".") {
        event.preventDefault();
        void store.stop();
      } else if (key === "backspace" && event.shiftKey) {
        event.preventDefault();
        store.clearAll();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  /*
   * ⌘V anywhere: copy a link, press paste, and the box is already filled in.
   *
   * This is the feature, not a shortcut for it — nobody opens a dialog to paste something they have
   * already copied. A `paste` listener rather than a ⌘V key binding, because only the event carries
   * the clipboard, and the browser will not hand it over on request without a permission prompt.
   *
   * Three things it must not steal it from: the box's own textarea (where ⌘V is how the link gets
   * *in*), the settings sheet's inputs (a pasted folder path is not a video), and the window while
   * the install question is up — `openLinks` refuses in that state anyway, but a swallowed ⌘V would
   * leave the user's paste nowhere at all. The test for "is this a link" is the *backend's* host
   * list, so this and Rust cannot disagree about what counts; with support not yet loaded nothing is
   * claimed from the clipboard.
   */
  useEffect(() => {
    const onPaste = (event: ClipboardEvent): void => {
      const store = useStore.getState();
      if (store.cropOpen) return;
      if (store.linksOpen || store.drawerOpen || store.installPrompt !== null) return;
      // Same for the sign-in question and its walkthrough: `openLinks` refuses while either is up,
      // and a ⌘V swallowed here would leave the user's paste nowhere at all.
      if (store.signInPrompt !== null || store.signInOpen) return;
      if (isTyping(event.target)) return;
      const hosts = store.linkSupport?.accepted_hosts ?? [];
      if (hosts.length === 0) return;
      const text = event.clipboardData?.getData("text") ?? "";
      if (text.trim() === "") return;
      const lower = text.toLowerCase();
      if (!hosts.some((host) => lower.includes(host)) && !lower.includes(".bandcamp.com/")) return;
      event.preventDefault();
      store.openLinks(text);
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, []);

  return (
    <div className="app" data-dragging={dragging || undefined} data-empty={!hasFiles || undefined}>
      <TitleBar />

      <main className="main">{hasFiles ? <FileList /> : <DropZone />}</main>

      {(hasFiles || converting) && <ActionBar />}
      <SettingsDrawer />
      {/* The one modal: raised only when a batch settles having needed a helper nobody installed. */}
      <InstallPrompt />
      {/* The paste box, over the same window and by the same rules: only ever one of the two. */}
      <LinksSheet />
      {/* And the sign-in pair: the one question a batch stopped by a sign-in wall raises, and the
          walkthrough for when one click is not enough. Both obey the same "one sheet" rule. */}
      <SignInPrompt />
      <SignInSheet />

      {dragging && hasFiles && (
        <div className="dragveil" aria-hidden="true">
          <span className="dragveil__label">Drop to add</span>
        </div>
      )}

      {error !== null ? (
        <div className="toast" role="alert">
          <span className="toast__text">{error}</span>
          {initializationFailed && (
            <button type="button" className="microlink" onClick={() => void init()}>
              Retry
            </button>
          )}
          {!initializationFailed && (
            <button type="button" className="iconbutton" aria-label="Dismiss" onClick={clearError}>
              <span aria-hidden="true">×</span>
            </button>
          )}
        </div>
      ) : (
        /*
         * The same slot in a quieter hand: a statement of fact, not a failure — at present only a
         * drop the file cap cut short. `role="status"` rather than `alert`, no danger hairline, and
         * it interrupts nothing: the files that did land are already convertible behind it. An error
         * is the louder thing and keeps the slot to itself, since the two would sit on top of each
         * other.
         */
        notice !== null && (
          <div className="toast toast--quiet" role="status">
            <span className="toast__text">{notice}</span>
            <button type="button" className="iconbutton" aria-label="Dismiss" onClick={clearNotice}>
              <span aria-hidden="true">×</span>
            </button>
          </div>
        )
      )}
    </div>
  );
}

/**
 * Settings, reachable from the toolbar or menu bar (⌘,).
 *
 * A right-hand sheet over a dimmed, blurred backdrop rather than a modal: the queue stays visible
 * behind it, because changing a setting is something you do *while* looking at what you are about
 * to convert. Esc and a click on the backdrop close it, and focus goes back where it came from.
 */
import { useEffect, useLayoutEffect, useRef } from "react";
import type { CSSProperties } from "react";
import { ArrowLeft, Palette } from "lucide-react";
import { DEFAULT_SKIN, skinVariables } from "../lib/skin";
import { useStore } from "../state/store";
import { SkinEditor } from "./SkinEditor";
import { PresetCards } from "./PresetCards";
import { SettingSections } from "./SettingCards";
import { ToolsStrip } from "./ToolsStrip";
import { WarningBanner } from "./WarningBanner";

const EDITOR_STYLE: CSSProperties = {
  ...skinVariables(DEFAULT_SKIN, "light"),
  fontFamily: skinVariables(DEFAULT_SKIN, "light")["--sans"],
  colorScheme: "light",
};

export function SettingsDrawer(): React.JSX.Element {
  const open = useStore((s) => s.drawerOpen);
  const page = useStore((s) => s.drawerPage);
  const openSkin = useStore((s) => s.openSkin);
  const showSettings = useStore((s) => s.showConversionSettings);
  const setDrawer = useStore((s) => s.setDrawer);
  const reset = useStore((s) => s.resetSettings);
  const closeRef = useRef<HTMLButtonElement>(null);
  const sheetRef = useRef<HTMLElement>(null);
  /** Whatever had focus when the sheet opened, so Esc can hand it back. */
  const opener = useRef<HTMLElement | null>(null);
  const pageRef = useRef(page);
  useLayoutEffect(() => { if (sheetRef.current) sheetRef.current.inert = !open; }, [open]);

  useEffect(() => {
    if (open) {
      const active = document.activeElement;
      opener.current = active instanceof HTMLElement ? active : null;
      // The panel may still be translated off-screen during its entrance.
      // Scrolling it into view would scroll the app's clipped container sideways.
      closeRef.current?.focus({ preventScroll: true });
      return;
    }
    // Closing makes the whole sheet `inert`; without this the focus ring falls off to <body> and
    // the next Tab starts from the top of the window.
    const previous = opener.current;
    opener.current = null;
    if (previous !== null && previous.isConnected) previous.focus({ preventScroll: true });
  }, [open]);

  useEffect(() => {
    if (pageRef.current !== page && open) closeRef.current?.focus({ preventScroll: true });
    pageRef.current = page;
  }, [page, open]);

  /**
   * Tab cycles inside the sheet. The scrim already swallows every click aimed at the queue, so
   * letting Tab walk out to controls that cannot be clicked — Convert, the row pickers — would
   * hand the keyboard a set of actions the mouse cannot reach.
   */
  const onKeyDownTrap = (event: React.KeyboardEvent): void => {
    if (event.key !== "Tab" || sheetRef.current === null) return;
    const focusable = Array.from(
      sheetRef.current.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      ),
    ).filter((el) => !el.hasAttribute("disabled") && el.tabIndex !== -1);
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (first === undefined || last === undefined) return;
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === sheetRef.current)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <>
      <div
        className="scrim"
        style={page === "skin" ? {
          background: "transparent", backdropFilter: "none", WebkitBackdropFilter: "none",
        } : undefined}
        data-open={open || undefined}
        aria-hidden="true"
        onClick={() => setDrawer(false)}
      />

      <aside
        ref={sheetRef}
        className="drawer"
        style={page === "skin" ? EDITOR_STYLE : undefined}
        data-open={open || undefined}
        aria-label="Settings"
        aria-hidden={!open}
        // Nothing inside is reachable by keyboard while the sheet is off-screen.
        inert={!open}
        onKeyDown={onKeyDownTrap}
      >
        <header className="drawer__header">
          <h2 className="drawer__title">{page === "skin" ? "Skin" : "Settings"}</h2>
          <div className="drawer__actions">
            {page === "skin" ? (
              <button type="button" className="iconbutton" aria-label="Back to settings"
                title="Back to settings" onClick={showSettings}>
                <ArrowLeft size={16} aria-hidden="true" />
              </button>
            ) : (
              <button type="button" className="iconbutton" aria-label="Customize skin"
                title="Customize skin" onClick={openSkin}>
                <Palette size={16} aria-hidden="true" />
              </button>
            )}
            <button
              ref={closeRef}
              type="button"
              className="iconbutton"
              aria-label="Close settings"
              onClick={() => setDrawer(false)}
            >
              <span aria-hidden="true">×</span>
            </button>
          </div>
        </header>

        <div className="drawer__body">
          {page === "skin" && open ? <SkinEditor /> : page === "conversion" ? (
            <>
              <WarningBanner />
              <PresetCards />
              <SettingSections />
              <ToolsStrip />
            </>
          ) : null}
        </div>

        {page === "conversion" && (
          <footer className="drawer__footer">
            <button type="button" className="microlink" onClick={() => void reset()}>
              Reset to defaults
            </button>
          </footer>
        )}
      </aside>
    </>
  );
}

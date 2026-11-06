/** Window drag region and persistent entry points for adding sources and settings. */
import { FilePlus2, Link, Settings } from "lucide-react";
import { pluralFiles } from "../lib/format";
import { useStore } from "../state/store";

export function TitleBar(): React.JSX.Element {
  const count = useStore((s) => s.order.length);
  const busy = useStore((s) => s.busy);
  const covered = useStore((s) => s.drawerOpen || s.linksOpen || s.signInOpen || s.cropOpen ||
    s.installPrompt !== null || s.signInPrompt !== null);
  const openFiles = useStore((s) => s.openFiles);
  const openLinks = useStore((s) => s.openLinks);
  const setDrawer = useStore((s) => s.setDrawer);

  return (
    <header className="titlebar" data-tauri-drag-region>
      {count > 0 && (
        <>
          <span className="titlebar__mark" data-tauri-drag-region>Flint</span>
          <span className="titlebar__count" data-tauri-drag-region>{count} {pluralFiles(count)}</span>
        </>
      )}
      <nav className="titlebar__actions" aria-label="Queue actions" aria-hidden={covered || undefined}>
        <button type="button" className="titlebar__button" aria-label="Add files"
          tabIndex={covered ? -1 : 0}
          title="Add files (⌘/Ctrl+O)" disabled={busy} onClick={(event) => {
            event.currentTarget.focus(); void openFiles();
          }}>
          <FilePlus2 size={14} aria-hidden="true" /><span>Add files</span>
        </button>
        <button type="button" className="titlebar__button" aria-label="Paste links"
          tabIndex={covered ? -1 : 0}
          title="Paste links (⌘/Ctrl+L)" onClick={(event) => {
            event.currentTarget.focus(); openLinks();
          }}>
          <Link size={14} aria-hidden="true" /><span>Paste links</span>
        </button>
        <button type="button" className="titlebar__button" aria-label="Open settings"
          tabIndex={covered ? -1 : 0}
          title="Settings (⌘/Ctrl+,)" onClick={(event) => {
            event.currentTarget.focus(); setDrawer(true);
          }}>
          <Settings size={14} aria-hidden="true" /><span>Settings</span>
        </button>
      </nav>
    </header>
  );
}

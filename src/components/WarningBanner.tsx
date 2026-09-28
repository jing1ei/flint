/**
 * The one thing worth interrupting for: a missing FFmpeg sidecar, which breaks nearly every
 * conversion. This is the whole of it: the explanation, the retry and the dismissal, all inside the
 * settings sheet, where there is room to say what went wrong and what to do about it. The empty
 * canvas states the same fact in one line (`DropZone`), because a first-run user with a broken
 * install never opens Settings — and dismissing it here dismisses both.
 *
 * It disappears the moment "Check again" finds the engine.
 */
import { engineMissing, useStore } from "../state/store";

export function WarningBanner(): React.JSX.Element | null {
  const missing = useStore(engineMissing);
  const dismiss = useStore((s) => s.dismissBanner);
  const refresh = useStore((s) => s.refreshTools);

  if (!missing) return null;

  return (
    <div className="notice" role="status">
      <p className="notice__text">
        The bundled FFmpeg engine could not be found, so most conversions will fail. Reinstalling
        the app usually fixes it.
      </p>
      <div className="notice__actions">
        <button type="button" className="microlink" onClick={() => void refresh()}>
          Check again
        </button>
        <button type="button" className="microlink" aria-label="Dismiss warning" onClick={dismiss}>
          Dismiss
        </button>
      </div>
    </div>
  );
}

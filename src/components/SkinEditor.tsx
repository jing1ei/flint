import { useEffect, useRef, useState } from "react";
import { Check, Copy, Eye, RotateCcw, Undo2, WandSparkles } from "lucide-react";
import { copyText } from "../lib/clipboard";
import { DEFAULT_SKIN, MAX_SKIN_LENGTH, parseSkin, skinCode, skinPrompt } from "../lib/skin";
import { currentSkinCode, useSkin } from "../state/skin";

export function SkinEditor(): React.JSX.Element {
  const [code, setCode] = useState(currentSkinCode);
  const [copying, setCopying] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [copyError, setCopyError] = useState<string | null>(null);
  const [seconds, setSeconds] = useState(20);
  const box = useRef<HTMLTextAreaElement>(null);
  const alive = useRef(true);
  const preview = useSkin((state) => state.preview);
  const deadline = useSkin((state) => state.deadline);
  const error = useSkin((state) => state.error);
  const notice = useSkin((state) => state.notice);
  const saved = useSkin((state) => state.saved);
  const beginPreview = useSkin((state) => state.beginPreview);
  const keep = useSkin((state) => state.keep);
  const revert = useSkin((state) => state.revert);
  const reset = useSkin((state) => state.reset);
  const clearMessage = useSkin((state) => state.clearMessage);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      // Leaving the editor is not consent to keep an unconfirmed skin.
      useSkin.getState().revert();
    };
  }, []);

  useEffect(() => {
    if (deadline === null) return undefined;
    const tick = (): void => setSeconds(Math.max(0, Math.ceil((deadline - Date.now()) / 1000)));
    tick();
    const timer = setInterval(tick, 250);
    return () => clearInterval(timer);
  }, [deadline]);

  const copy = async (prompt: boolean): Promise<void> => {
    setCopying(true);
    setMessage(null);
    setCopyError(null);
    try {
      const text = prompt ? skinPrompt(parseSkin(code)) : code;
      const ok = await copyText(text);
      if (!alive.current) return;
      if (ok) setMessage(prompt ? "LLM prompt and skin copied." : "Skin code copied.");
      else {
        box.current?.focus();
        box.current?.select();
        setCopyError("Clipboard unavailable. Select and copy the skin code manually.");
      }
    } catch (e) {
      if (alive.current) setCopyError(e instanceof Error ? e.message : "Could not copy skin.");
    } finally {
      if (alive.current) setCopying(false);
    }
  };

  return (
    <section className="skin" aria-label="Skin customization">
      <div className="skin__heading">
        <span>Skin code</span>
        <span className="skin__name" title={saved?.name ?? "Original"}>{saved?.name ?? "Original"}</span>
      </div>
      <div className="skin__tools">
        <button type="button" className="skin__button" title="Copy skin code" disabled={copying}
          onClick={() => void copy(false)}>
          <Copy size={14} aria-hidden="true" /> Copy code
        </button>
        <button type="button" className="skin__button" title="Copy LLM prompt and current skin" disabled={copying}
          onClick={() => void copy(true)}>
          <WandSparkles size={14} aria-hidden="true" /> Copy LLM prompt
        </button>
      </div>
      <textarea ref={box} className="skin__code" aria-label="Skin JSON code"
        aria-invalid={error !== null} aria-describedby="skin-message"
        spellCheck={false} autoCapitalize="off" autoCorrect="off"
        maxLength={MAX_SKIN_LENGTH + 1} value={code} disabled={preview !== null}
        onChange={(event) => {
          setCode(event.target.value);
          setMessage(null);
          setCopyError(null);
          clearMessage();
        }} />
      <div className="skin__message" id="skin-message">
        {error !== null || copyError !== null ? (
          <span role="alert">{error ?? copyError}</span>
        ) : (
          <span role="status">{message ?? notice ?? "\u00a0"}</span>
        )}
        {deadline !== null && <span aria-hidden="true">Reverts in {seconds}s</span>}
      </div>
      <div className="skin__actions">
        {preview === null ? (
          <button type="button" className="skin__button" disabled={code.trim() === ""}
            onClick={() => {
              setMessage(null);
              setCopyError(null);
              beginPreview(code);
            }}>
            <Eye size={14} aria-hidden="true" /> Preview skin
          </button>
        ) : (
          <>
            <button type="button" className="skin__button" onClick={keep}>
              <Check size={14} aria-hidden="true" /> Keep skin
            </button>
            <button type="button" className="skin__button" onClick={revert}>
              <Undo2 size={14} aria-hidden="true" /> Revert
            </button>
          </>
        )}
        <button type="button" className="skin__button" title="Restore original colors and fonts"
          onClick={() => {
            reset();
            setCode(skinCode(DEFAULT_SKIN));
            setMessage(null);
            setCopyError(null);
          }}>
          <RotateCcw size={14} aria-hidden="true" /> Reset skin
        </button>
      </div>
    </section>
  );
}

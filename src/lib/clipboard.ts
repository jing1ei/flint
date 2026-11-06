/**
 * Copy one line of text, with a fallback.
 *
 * The async Clipboard API needs a secure context and a permission the webview may refuse, and the
 * command next to a Copy button is the *fallback* for everything else in this app — so it must not
 * be the thing that quietly does nothing. `execCommand("copy")` is deprecated and works everywhere
 * that matters, which is exactly what a fallback is for.
 */
export async function copyText(text: string): Promise<boolean> {
  if (text === "") return false;
  try {
    if (typeof navigator !== "undefined" && navigator.clipboard !== undefined) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Permission refused or no secure context: fall through to the selection trick.
  }
  return legacyCopy(text);
}

function legacyCopy(text: string): boolean {
  if (typeof document === "undefined") return false;
  const previous = document.activeElement;
  const field = document.createElement("textarea");
  field.value = text;
  // Off-screen rather than `display: none`: a hidden field cannot be selected, and a visible one
  // would flash a box over the settings sheet.
  field.setAttribute("aria-hidden", "true");
  field.style.position = "fixed";
  field.style.top = "-1000px";
  field.style.opacity = "0";
  document.body.append(field);
  field.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  field.remove();
  if (previous instanceof HTMLElement && previous.isConnected) previous.focus({ preventScroll: true });
  return ok;
}

// WebView2 marks the focused button :focus-visible after any key, including Shift for a screenshot,
// so focus rings are shown only after Tab and hidden again on the next pointer press.
export function trackKeyboardFocus(root: HTMLElement = document.documentElement): () => void {
  const onKey = (event: KeyboardEvent) => {
    if (event.key === "Tab") root.classList.add("keyboard-nav");
  };
  const onPointer = () => root.classList.remove("keyboard-nav");
  window.addEventListener("keydown", onKey, true);
  window.addEventListener("pointerdown", onPointer, true);
  return () => {
    window.removeEventListener("keydown", onKey, true);
    window.removeEventListener("pointerdown", onPointer, true);
  };
}

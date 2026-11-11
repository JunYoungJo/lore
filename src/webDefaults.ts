// The windows are app UI, not web pages. A webview still offers a page's behaviors (browser menu, page zoom,
// reload and find shortcuts, dragging content out, spelling marks), so every window cancels them here.
// WebView2 has native switches for some of these (see harden_webview); this covers every platform and window.

const PAGE_KEYS = new Set(["F3", "F5", "F7"]);
// Reload, find, find next, print, view source, save page, downloads, open file, zoom in, zoom out, reset zoom.
const COMMAND_KEYS = new Set(["r", "f", "g", "p", "u", "s", "j", "o", "=", "+", "-", "0"]);
const HISTORY_KEYS_MAC = new Set(["[", "]"]);
const HISTORY_KEYS_OTHER = new Set(["ArrowLeft", "ArrowRight"]);
const FIELD_DEFAULTS: [string, string][] = [["spellcheck", "false"], ["autocomplete", "off"], ["autocorrect", "off"], ["autocapitalize", "off"]];

export function suppressWebDefaults(target: Window = window, platform: string = navigator.platform): () => void {
  const mac = platform.startsWith("Mac");
  const cancel = (event: Event) => event.preventDefault();

  const onMouseDown = (event: MouseEvent) => {
    // The middle button starts the browser's scroll-by-moving mode.
    if (event.button === 1) event.preventDefault();
  };
  const onWheel = (event: WheelEvent) => {
    // Ctrl+wheel zooms the page; a trackpad pinch arrives the same way.
    if (event.ctrlKey) event.preventDefault();
  };
  const onKey = (event: KeyboardEvent) => {
    // macOS text fields move the cursor with Control+F/P/..., so only Command counts there.
    const command = mac ? event.metaKey : event.ctrlKey;
    const history = mac ? event.metaKey && HISTORY_KEYS_MAC.has(event.key) : event.altKey && HISTORY_KEYS_OTHER.has(event.key);
    if (PAGE_KEYS.has(event.key) || (command && COMMAND_KEYS.has(event.key.toLowerCase())) || history) event.preventDefault();
  };
  const onFocus = (event: FocusEvent) => {
    const field = event.target;
    if (!(field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement)) return;
    for (const [name, value] of FIELD_DEFAULTS) if (!field.hasAttribute(name)) field.setAttribute(name, value);
  };

  const listeners: [string, EventListener, AddEventListenerOptions][] = [
    ["contextmenu", cancel, { capture: true }],
    ["dragstart", cancel, { capture: true }],
    ["gesturestart", cancel, { capture: true }],
    ["mousedown", onMouseDown as EventListener, { capture: true }],
    ["wheel", onWheel as EventListener, { capture: true, passive: false }],
    ["keydown", onKey as EventListener, { capture: true }],
    ["focusin", onFocus as EventListener, { capture: true }],
  ];
  for (const [type, listener, options] of listeners) target.addEventListener(type, listener, options);
  return () => {
    for (const [type, listener, options] of listeners) target.removeEventListener(type, listener, options);
  };
}

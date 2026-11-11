import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { suppressWebDefaults } from "@/webDefaults";

let stop: (() => void) | null = null;

afterEach(() => {
  stop?.();
  stop = null;
  document.body.innerHTML = "";
});

// True when the page cancelled the browser's own handling of the event.
function cancelled(event: Event, target: EventTarget = document.body): boolean {
  target.dispatchEvent(event);
  return event.defaultPrevented;
}

const key = (name: string, modifiers: KeyboardEventInit = {}) => new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true, ...modifiers });

describe("browser behaviors a webview still offers", () => {
  it("opens no browser menu on right click, on plain UI or in a field", () => {
    stop = suppressWebDefaults();
    document.body.innerHTML = "<p>Note</p><input />";
    expect(cancelled(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }), document.querySelector("p")!)).toBe(true);
    expect(cancelled(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }), document.querySelector("input")!)).toBe(true);
  });

  it("does not drag text, pictures or links out of the window", () => {
    stop = suppressWebDefaults();
    expect(cancelled(new Event("dragstart", { bubbles: true, cancelable: true }))).toBe(true);
  });

  it("does not start middle-button scrolling but leaves ordinary clicks alone", () => {
    stop = suppressWebDefaults();
    expect(cancelled(new MouseEvent("mousedown", { button: 1, bubbles: true, cancelable: true }))).toBe(true);
    expect(cancelled(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }))).toBe(false);
    expect(cancelled(new MouseEvent("mousedown", { button: 2, bubbles: true, cancelable: true }))).toBe(false);
  });

  it("does not zoom the page with the wheel or a pinch but still scrolls", () => {
    stop = suppressWebDefaults();
    expect(cancelled(new WheelEvent("wheel", { deltaY: -100, ctrlKey: true, bubbles: true, cancelable: true }))).toBe(true);
    expect(cancelled(new WheelEvent("wheel", { deltaY: -100, bubbles: true, cancelable: true }))).toBe(false);
    expect(cancelled(new Event("gesturestart", { bubbles: true, cancelable: true }))).toBe(true);
  });

  it("ignores reload, find, print, zoom and history shortcuts on Windows and Linux", () => {
    stop = suppressWebDefaults(window, "Win32");
    for (const name of ["F3", "F5", "F7"]) expect(cancelled(key(name)), name).toBe(true);
    for (const name of ["r", "R", "f", "g", "p", "u", "s", "j", "o", "=", "+", "-", "0"]) expect(cancelled(key(name, { ctrlKey: true })), `Ctrl+${name}`).toBe(true);
    for (const name of ["ArrowLeft", "ArrowRight"]) expect(cancelled(key(name, { altKey: true })), `Alt+${name}`).toBe(true);
  });

  it("keeps editing shortcuts and ordinary typing", () => {
    stop = suppressWebDefaults(window, "Win32");
    for (const name of ["c", "v", "x", "a", "z"]) expect(cancelled(key(name, { ctrlKey: true })), `Ctrl+${name}`).toBe(false);
    for (const name of ["r", "f", "0", "ArrowLeft", "Tab", "Enter", "F2"]) expect(cancelled(key(name)), name).toBe(false);
  });

  it("uses Command on macOS and leaves the Control cursor keys of text fields alone", () => {
    stop = suppressWebDefaults(window, "MacIntel");
    for (const name of ["r", "f", "g", "p", "=", "-", "0", "[", "]"]) expect(cancelled(key(name, { metaKey: true })), `Cmd+${name}`).toBe(true);
    for (const name of ["f", "p", "a", "e"]) expect(cancelled(key(name, { ctrlKey: true })), `Ctrl+${name}`).toBe(false);
    for (const name of ["c", "v", "a", "q", "w"]) expect(cancelled(key(name, { metaKey: true })), `Cmd+${name}`).toBe(false);
  });

  it("turns off spelling marks and autofill in a field when it is first used", () => {
    stop = suppressWebDefaults();
    document.body.innerHTML = '<input id="plain" /><textarea id="area"></textarea><input id="chosen" spellcheck="true" autocomplete="name" />';
    for (const id of ["plain", "area", "chosen"]) document.getElementById(id)!.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    for (const id of ["plain", "area"]) {
      const field = document.getElementById(id)!;
      expect(field.getAttribute("spellcheck")).toBe("false");
      expect(field.getAttribute("autocomplete")).toBe("off");
      expect(field.getAttribute("autocorrect")).toBe("off");
      expect(field.getAttribute("autocapitalize")).toBe("off");
    }
    const chosen = document.getElementById("chosen")!;
    expect(chosen.getAttribute("spellcheck")).toBe("true");
    expect(chosen.getAttribute("autocomplete")).toBe("name");
  });

  it("stops when cleaned up", () => {
    suppressWebDefaults()();
    expect(cancelled(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }))).toBe(false);
  });
});

describe("style sheets", () => {
  const folder = join(process.cwd(), "src/styles");
  const sheets = readdirSync(folder).filter((name) => name.endsWith(".css")).map((name) => [name, readFileSync(join(folder, name), "utf8")] as const);

  // WKWebView on macOS only honors the prefixed property; without it every label in a window can be selected.
  it("give every user-select rule its -webkit- form", () => {
    for (const [name, css] of sheets) {
      for (const rule of css.split("}")) {
        const plain = [...rule.matchAll(/(?<![-\w])user-select:\s*([a-z]+)/g)].map((match) => match[1]);
        const prefixed = [...rule.matchAll(/-webkit-user-select:\s*([a-z]+)/g)].map((match) => match[1]);
        expect(prefixed, `${name}: ${rule.trim().split("{")[0].trim()}`).toEqual(plain);
      }
    }
  });
});

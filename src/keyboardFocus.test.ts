import { afterEach, describe, expect, it } from "vitest";

import { trackKeyboardFocus } from "@/keyboardFocus";

let stop: (() => void) | null = null;

afterEach(() => {
  stop?.();
  stop = null;
  document.documentElement.classList.remove("keyboard-nav");
});

describe("keyboard focus rings", () => {
  it("shows them after Tab only, not after modifier keys, and hides them on the next pointer press", () => {
    stop = trackKeyboardFocus();
    const root = document.documentElement;
    for (const key of ["Shift", "Control", "Alt", "Meta", "Escape"]) {
      window.dispatchEvent(new KeyboardEvent("keydown", { key }));
      expect(root).not.toHaveClass("keyboard-nav");
    }
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true }));
    expect(root).toHaveClass("keyboard-nav");
    window.dispatchEvent(new Event("pointerdown"));
    expect(root).not.toHaveClass("keyboard-nav");
  });

  it("stops listening when cleaned up", () => {
    trackKeyboardFocus()();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
    expect(document.documentElement).not.toHaveClass("keyboard-nav");
  });
});

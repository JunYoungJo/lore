import { useEffect, useState } from "react";

export type Theme = "light" | "dark";

const KEY = "lore.theme";

function initialTheme(): Theme {
  try {
    const saved = localStorage.getItem(KEY);
    if (saved === "light" || saved === "dark") return saved;
  } catch {
    // Storage can be unavailable; fall back to the system preference.
  }
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function useTheme(): [Theme, () => void] {
  const [theme, setTheme] = useState<Theme>(initialTheme);
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    try {
      localStorage.setItem(KEY, theme);
    } catch {
      // Not persisting the choice only costs the next launch its theme.
    }
  }, [theme]);
  return [theme, () => setTheme((current) => (current === "dark" ? "light" : "dark"))];
}

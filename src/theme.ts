import { useEffect, useSyncExternalStore } from "react";

export type Theme = "light" | "dark";
export type ThemePreference = Theme | "system";

const KEY = "lore.theme";
const listeners = new Set<() => void>();

function readPreference(): ThemePreference {
  try {
    const saved = localStorage.getItem(KEY);
    if (saved === "light" || saved === "dark" || saved === "system") return saved;
  } catch {
    // Storage can be unavailable; follow the system appearance.
  }
  return "system";
}

let preference = readPreference();

const notify = () => listeners.forEach((listener) => listener());
const systemQuery = () => window.matchMedia?.("(prefers-color-scheme: dark)");
const resolved = (): Theme => (preference === "system" ? (systemQuery()?.matches ? "dark" : "light") : preference);

window.addEventListener("storage", (event) => {
  if (event.key !== KEY) return;
  preference = readPreference();
  notify();
});

function subscribe(listener: () => void) {
  listeners.add(listener);
  const query = systemQuery();
  query?.addEventListener?.("change", listener);
  return () => {
    listeners.delete(listener);
    query?.removeEventListener?.("change", listener);
  };
}

export function setThemePreference(next: ThemePreference) {
  preference = next;
  try {
    localStorage.setItem(KEY, next);
  } catch {
    // Not persisting the choice only costs the next launch its theme.
  }
  notify();
}

export function useThemePreference(): [ThemePreference, (next: ThemePreference) => void] {
  return [useSyncExternalStore(subscribe, () => preference), setThemePreference];
}

export function useTheme(): [Theme, () => void] {
  const theme = useSyncExternalStore(subscribe, resolved);
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);
  return [theme, () => setThemePreference(theme === "dark" ? "light" : "dark")];
}

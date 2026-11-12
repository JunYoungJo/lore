import type { Provider } from "@/ipc/contract";

export function relativeTime(seconds: number, now: number): string {
  const diff = Math.max(0, now - seconds);
  if (diff < 60) return "Just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86_400) return `${Math.floor(diff / 3600)}h ago`;
  const date = new Date(seconds * 1000);
  const today = new Date(now * 1000);
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  if (date >= yesterday) return "Yesterday";
  return new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric" }).format(date);
}

export function relativePhrase(seconds: number, now: number): string {
  const when = relativeTime(seconds, now);
  return when === "Just now" || when === "Yesterday" ? when.toLowerCase() : when;
}

export function elapsed(seconds: number, now: number): string {
  const diff = Math.max(0, now - seconds);
  if (diff < 60) return "Just started";
  return `${Math.floor(diff / 60)} min elapsed`;
}

export function minutesUntil(seconds: number, now: number): number {
  return Math.max(1, Math.ceil((seconds - now) / 60));
}

export function providerLabel(provider: Provider): string {
  return provider === "claude" ? "Claude" : "Codex";
}

/** The sentence of an activity summary. Its later lines name the notes that were removed or moved. */
export function summaryLine(summary: string | null): string {
  return (summary ?? "").split("\n")[0];
}

/** The notes an activity removed or moved away, each with the reason or where it went. */
export function goneNotes(summary: string | null): Map<string, string> {
  const gone = new Map<string, string>();
  for (const line of (summary ?? "").split("\n").slice(1)) {
    const removed = /^Removed (\S+): (.*)$/.exec(line);
    const moved = /^Moved (\S+) to (\S+)$/.exec(line);
    if (removed) gone.set(removed[1], removed[2]);
    else if (moved) gone.set(moved[1], `Moved to ${moved[2]}`);
  }
  return gone;
}

export function fileName(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] ?? path;
}

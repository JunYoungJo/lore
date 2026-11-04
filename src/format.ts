import type { Provider } from "@/ipc/contract";

export function relativeTime(seconds: number, now: number): string {
  const diff = Math.max(0, now - seconds);
  if (diff < 60) return "방금";
  if (diff < 3600) return `${Math.floor(diff / 60)}분 전`;
  if (diff < 86_400) return `${Math.floor(diff / 3600)}시간 전`;
  const date = new Date(seconds * 1000);
  const today = new Date(now * 1000);
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  if (date >= yesterday) return "어제";
  return `${date.getMonth() + 1}월 ${date.getDate()}일`;
}

export function elapsed(seconds: number, now: number): string {
  const diff = Math.max(0, now - seconds);
  if (diff < 60) return "방금 시작";
  return `${Math.floor(diff / 60)}분째`;
}

export function minutesUntil(seconds: number, now: number): number {
  return Math.max(1, Math.ceil((seconds - now) / 60));
}

export function providerLabel(provider: Provider): string {
  return provider === "claude" ? "Claude" : "Codex";
}

export function fileName(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] ?? path;
}

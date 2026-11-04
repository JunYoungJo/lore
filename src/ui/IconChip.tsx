import type { CSSProperties } from "react";
import type { LucideIcon } from "lucide-react";

export type Tint = "blue" | "green" | "amber" | "red" | "indigo" | "neutral";

const TINTS: Record<Tint, readonly [background: string, foreground: string]> = {
  blue: ["var(--accent-soft)", "var(--accent)"],
  green: ["var(--ok-soft)", "var(--ok)"],
  amber: ["var(--warn-soft)", "var(--warn)"],
  red: ["var(--danger-soft)", "var(--danger)"],
  indigo: ["color-mix(in srgb, #5b53c8 14%, var(--surface))", "#7a70e0"],
  neutral: ["var(--surface2)", "var(--text3)"],
};

export function IconChip({ glyph: Glyph, tint, size = 32, spin = false }: { glyph: LucideIcon; tint: Tint; size?: number; spin?: boolean }) {
  const [background, color] = TINTS[tint];
  const box: CSSProperties = {
    width: size,
    height: size,
    borderRadius: Math.round(size * 0.28),
    background,
    color,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    flex: "none",
  };
  return (
    <span style={box} aria-hidden="true">
      <Glyph size={Math.round(size * 0.5)} strokeWidth={1.9} className={spin ? "spin" : undefined} />
    </span>
  );
}

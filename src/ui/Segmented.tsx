import type { CSSProperties } from "react";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
}

export function Segmented<T extends string>({ value, options, onChange, label, compact = false }: { value: T; options: SegmentedOption<T>[]; onChange(value: T): void; label: string; compact?: boolean }) {
  const track: CSSProperties = { display: "flex", flex: "none", background: "var(--surface2)", borderRadius: compact ? 8 : 9, padding: compact ? 2 : 3, gap: 2 };
  return (
    <div style={track} role="group" aria-label={label}>
      {options.map((option) => {
        const active = option.value === value;
        const style: CSSProperties = {
          flex: 1,
          whiteSpace: "nowrap",
          border: "none",
          cursor: "pointer",
          fontSize: compact ? 11.5 : 12.5,
          padding: compact ? "3px 10px" : "6px 13px",
          borderRadius: compact ? 6 : 7,
          background: active ? "var(--surface)" : "transparent",
          color: active ? "var(--text)" : "var(--text2)",
          // A compact control is as wide as its labels, so a weight change would resize it on every switch.
          fontWeight: compact ? 500 : active ? 600 : 400,
          boxShadow: active ? "var(--sh1)" : "none",
        };
        return (
          <button key={option.value} type="button" aria-pressed={active} onClick={() => onChange(option.value)} style={style}>
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

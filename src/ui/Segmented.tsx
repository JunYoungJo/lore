import type { CSSProperties } from "react";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
}

export function Segmented<T extends string>({ value, options, onChange, label }: { value: T; options: SegmentedOption<T>[]; onChange(value: T): void; label: string }) {
  const track: CSSProperties = { display: "flex", background: "var(--surface2)", borderRadius: 9, padding: 3, gap: 2 };
  return (
    <div style={track} role="group" aria-label={label}>
      {options.map((option) => {
        const active = option.value === value;
        const style: CSSProperties = {
          flex: 1,
          whiteSpace: "nowrap",
          border: "none",
          cursor: "pointer",
          fontSize: 12.5,
          padding: "6px 13px",
          borderRadius: 7,
          background: active ? "var(--surface)" : "transparent",
          color: active ? "var(--text)" : "var(--text2)",
          fontWeight: active ? 600 : 400,
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

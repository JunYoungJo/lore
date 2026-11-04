import type { CSSProperties } from "react";

export function Toggle({ checked, onChange, label, disabled = false }: { checked: boolean; onChange(value: boolean): void; label: string; disabled?: boolean }) {
  const track: CSSProperties = {
    position: "relative",
    width: 42,
    height: 24,
    borderRadius: 999,
    border: "none",
    cursor: disabled ? "default" : "pointer",
    background: checked ? "var(--accent)" : "var(--line2)",
    transition: "background .18s",
    flex: "none",
    opacity: disabled ? 0.5 : 1,
  };
  const knob: CSSProperties = {
    position: "absolute",
    top: 3,
    left: 3,
    width: 18,
    height: 18,
    borderRadius: "50%",
    background: "#fff",
    boxShadow: "0 1px 2px rgba(0,0,0,.25)",
    transition: "transform .18s",
    transform: checked ? "translateX(18px)" : "translateX(0)",
  };
  return (
    <button type="button" role="switch" aria-checked={checked} aria-label={label} disabled={disabled} onClick={() => onChange(!checked)} style={track}>
      <span style={knob} />
    </button>
  );
}

import { useId } from "react";

export function BrandMark({ size = 28 }: { size?: number }) {
  const uid = useId().replace(/:/g, "");
  const bg = `${uid}bg`;
  return (
    <svg width={size} height={size} viewBox="0 0 512 512" aria-hidden="true" style={{ display: "block", flex: "none" }}>
      <defs>
        <linearGradient id={bg} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#4c8df0" />
          <stop offset="1" stopColor="#2253a0" />
        </linearGradient>
      </defs>
      <rect x="16" y="16" width="480" height="480" rx="108" fill={`url(#${bg})`} />
      <path d="M164 124h48v210a14 14 0 0 0 14 14h126v42H184a38 38 0 0 1-38-38V142a18 18 0 0 1 18-18Z" fill="#fff" />
      <path d="M241 139h91a20 20 0 0 1 20 20v157H241Z" fill="#fff" opacity=".82" />
      <path d="M241 139h31v79l-15.5-11-15.5 11Z" fill="#2c66ae" />
    </svg>
  );
}

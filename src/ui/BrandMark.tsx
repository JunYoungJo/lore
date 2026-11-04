import { useId } from "react";

export function BrandMark({ size = 28 }: { size?: number }) {
  const uid = useId().replace(/:/g, "");
  const bg = `${uid}bg`;
  const page = `${uid}page`;
  return (
    <svg width={size} height={size} viewBox="0 0 512 512" aria-hidden="true" style={{ display: "block", flex: "none" }}>
      <defs>
        <linearGradient id={bg} x1="0.15" y1="0" x2="0.85" y2="1">
          <stop offset="0" stopColor="#3B7BD0" />
          <stop offset="1" stopColor="#1D4A86" />
        </linearGradient>
        <linearGradient id={page} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#FFFFFF" />
          <stop offset="1" stopColor="#DCE8F9" />
        </linearGradient>
      </defs>
      <rect width="512" height="512" rx="118" fill={`url(#${bg})`} />
      <path d="M168 112h128l72 72v196a20 20 0 0 1-20 20H168a20 20 0 0 1-20-20V132a20 20 0 0 1 20-20z" fill={`url(#${page})`} />
      <path d="M296 112v52a20 20 0 0 0 20 20h52z" fill="#AFC9EE" />
      <rect x="190" y="232" width="132" height="22" rx="11" fill="#2C66AE" />
      <rect x="190" y="282" width="132" height="22" rx="11" fill="#2C66AE" opacity="0.75" />
      <rect x="190" y="332" width="84" height="22" rx="11" fill="#2C66AE" opacity="0.5" />
    </svg>
  );
}

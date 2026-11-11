import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";

const GAP = 6;
const EDGE = 8;

export function AnchoredCard({
  anchor,
  label,
  role,
  className,
  focus,
  onClose,
  children,
}: {
  anchor: RefObject<HTMLElement>;
  label: string;
  role: "dialog" | "alertdialog";
  className: string;
  focus: RefObject<HTMLElement>;
  onClose: () => void;
  children: ReactNode;
}) {
  const card = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    const place = () => {
      const target = anchor.current?.getBoundingClientRect();
      const box = card.current?.getBoundingClientRect();
      if (!target || !box) return;
      const below = target.bottom + GAP;
      const top = below + box.height <= window.innerHeight - EDGE ? below : Math.max(EDGE, target.top - GAP - box.height);
      const left = Math.min(Math.max(EDGE, target.right - box.width), window.innerWidth - box.width - EDGE);
      setPosition({ top, left });
    };
    place();
    window.addEventListener("scroll", place, true);
    return () => window.removeEventListener("scroll", place, true);
  }, [anchor]);

  useEffect(() => {
    focus.current?.focus();
    // Capture phase, so Escape closes only this card and never reaches the handler that hides the whole popover.
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      close.current();
      anchor.current?.focus();
    };
    const onPointer = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!card.current?.contains(target) && !anchor.current?.contains(target)) close.current();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("pointerdown", onPointer, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("pointerdown", onPointer, true);
    };
  }, [anchor, focus]);

  return createPortal(
    <div ref={card} role={role} aria-label={label} className={className} style={position ?? { visibility: "hidden" }}>
      {children}
    </div>,
    document.body,
  );
}

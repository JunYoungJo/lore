import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";

const GAP = 6;
const EDGE = 8;

export function ConfirmCard({
  anchor,
  label,
  message,
  action,
  onConfirm,
  onClose,
}: {
  anchor: RefObject<HTMLElement>;
  label: string;
  message: string;
  action: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const card = useRef<HTMLDivElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);

  const dismiss = () => {
    close.current();
    anchor.current?.focus();
  };

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
    cancel.current?.focus();
    // Capture phase, so Escape closes only this card and never reaches the handler that hides the whole popover.
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      dismiss();
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
  }, [anchor]);

  return createPortal(
    <div ref={card} role="alertdialog" aria-label={label} className="confirm-card" style={position ?? { visibility: "hidden" }}>
      <p className="confirm-text">{message}</p>
      <div className="confirm-actions">
        <button ref={cancel} type="button" className="pill ghost" onClick={dismiss}>
          취소
        </button>
        <button
          type="button"
          className="pill danger"
          onClick={() => {
            close.current();
            onConfirm();
          }}
        >
          {action}
        </button>
      </div>
    </div>,
    document.body,
  );
}

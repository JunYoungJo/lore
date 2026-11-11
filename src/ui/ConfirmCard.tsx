import { useRef, type RefObject } from "react";

import { AnchoredCard } from "@/ui/AnchoredCard";

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
  const cancel = useRef<HTMLButtonElement>(null);
  return (
    <AnchoredCard anchor={anchor} label={label} role="alertdialog" className="confirm-card" focus={cancel} onClose={onClose}>
      <p className="confirm-text">{message}</p>
      <div className="confirm-actions">
        <button
          ref={cancel}
          type="button"
          className="pill ghost"
          onClick={() => {
            onClose();
            anchor.current?.focus();
          }}
        >
          Cancel
        </button>
        <button
          type="button"
          className="pill danger"
          onClick={() => {
            onClose();
            onConfirm();
          }}
        >
          {action}
        </button>
      </div>
    </AnchoredCard>
  );
}

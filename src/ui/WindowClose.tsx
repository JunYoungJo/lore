import { X } from "lucide-react";

export function WindowClose({ onClose, disabled = false }: { onClose: () => void; disabled?: boolean }) {
  return (
    <button type="button" className="window-close" disabled={disabled} onClick={onClose}>
      Close <X size={14} strokeWidth={2} />
    </button>
  );
}

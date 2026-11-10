import { useRef, useState } from "react";

import type { Job } from "@/ipc/contract";
import { useAppStore } from "@/store/appStore";
import { ConfirmCard } from "@/ui/ConfirmCard";

export function JobActions({ job }: { job: Job }) {
  const retry = useAppStore((store) => store.retry);
  const discard = useAppStore((store) => store.discard);
  const [confirming, setConfirming] = useState(false);
  const discardButton = useRef<HTMLButtonElement>(null);
  return (
    <div className="row-actions">
      <button ref={discardButton} type="button" className="pill ghost" aria-expanded={confirming} onClick={() => setConfirming(!confirming)}>
        Skip
      </button>
      <button type="button" className="pill primary" onClick={() => void retry(job.id)}>
        Retry
      </button>
      {confirming && (
        <ConfirmCard
          anchor={discardButton}
          label="Skip failed segment confirmation"
          message="Skip the failed segment? If the source cannot be read, recording will stop for this session."
          action="Skip"
          onConfirm={() => void discard(job.id)}
          onClose={() => setConfirming(false)}
        />
      )}
    </div>
  );
}

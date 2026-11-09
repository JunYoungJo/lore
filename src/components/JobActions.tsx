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
        버리기
      </button>
      <button type="button" className="pill primary" onClick={() => void retry(job.id)}>
        다시 시도
      </button>
      {confirming && (
        <ConfirmCard
          anchor={discardButton}
          label="실패 버리기 확인"
          message="실패한 구간을 기록하지 않고 넘어갈까요? 원문을 읽을 수 없는 실패면 이 세션은 더 기록하지 않습니다."
          action="버리기"
          onConfirm={() => void discard(job.id)}
          onClose={() => setConfirming(false)}
        />
      )}
    </div>
  );
}

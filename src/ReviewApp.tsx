import { useCallback, useEffect, useRef, useState } from "react";
import { FileMinus, FilePen, FilePlus, GitCommitHorizontal, LoaderCircle, TriangleAlert, type LucideIcon } from "lucide-react";

import { DiffView } from "@/components/DiffView";
import * as api from "@/ipc/api";
import type { ChangeKind, DiffRow, ReviewFile, ReviewSnapshot } from "@/ipc/contract";
import { fileName, relativeTime } from "@/format";
import { useTheme } from "@/theme";
import { useNow } from "@/useNow";

const KIND: Record<ChangeKind, { label: string; glyph: LucideIcon }> = {
  modified: { label: "수정", glyph: FilePen },
  added: { label: "새 문서", glyph: FilePlus },
  deleted: { label: "삭제", glyph: FileMinus },
};

function folderOf(path: string): string {
  return path.slice(0, path.lastIndexOf("/"));
}

function byProject(files: ReviewFile[]): { folder: string; files: ReviewFile[] }[] {
  const groups = new Map<string, ReviewFile[]>();
  for (const file of files) {
    const folder = folderOf(file.path);
    groups.set(folder, [...(groups.get(folder) ?? []), file]);
  }
  return [...groups].map(([folder, members]) => ({ folder, files: members }));
}

function Check({ label, checked, mixed = false, onChange }: { label: string; checked: boolean; mixed?: boolean; onChange: (checked: boolean) => void }) {
  const box = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (box.current) box.current.indeterminate = mixed;
  }, [mixed]);
  return <input ref={box} type="checkbox" className="check" aria-label={label} checked={checked} onChange={(event) => onChange(event.target.checked)} />;
}

function GroupCheck({ label, paths, checked, onChange }: { label: string; paths: string[]; checked: ReadonlySet<string>; onChange: (paths: string[], checked: boolean) => void }) {
  const count = paths.filter((path) => checked.has(path)).length;
  return <Check label={label} checked={count > 0 && count === paths.length} mixed={count > 0 && count < paths.length} onChange={(next) => onChange(paths, next)} />;
}

function FileItem({ file, viewing, checked, now, onView, onCheck }: { file: ReviewFile; viewing: boolean; checked: boolean; now: number; onView: () => void; onCheck: (checked: boolean) => void }) {
  const kind = KIND[file.kind];
  const Glyph = kind.glyph;
  return (
    <li className={`review-item ${viewing ? "active" : ""}`}>
      <Check label={`${file.path} 선택`} checked={checked} onChange={onCheck} />
      <button type="button" className="review-item-pick" aria-current={viewing} onClick={onView}>
        <div className="review-item-top">
          <Glyph size={15} strokeWidth={1.9} className={`kind-${file.kind}`} />
          <span className="review-item-name" title={file.path}>
            {fileName(file.path)}
          </span>
          <span className={`badge kind-${file.kind}`}>{kind.label}</span>
        </div>
        {file.sessions.map((session) => (
          <div key={`${session.at}-${session.project}`} className="review-session" title={session.summary ?? undefined}>
            <span>
              {session.project ?? session.provider} · {(session.summary ?? "").split("\n")[0]}
            </span>
            <span className="review-session-time">{relativeTime(session.at, now)}</span>
          </div>
        ))}
      </button>
    </li>
  );
}

export function ReviewApp() {
  useTheme();
  const now = useNow(15_000);
  const [review, setReview] = useState<ReviewSnapshot | null>(null);
  const [viewing, setViewing] = useState<string | null>(null);
  const [checked, setChecked] = useState<ReadonlySet<string>>(new Set());
  const [rows, setRows] = useState<DiffRow[] | null>(null);
  const [message, setMessage] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [committing, setCommitting] = useState(false);
  const waiting = useRef(true);
  const messageLoaded = useRef(false);

  const load = useCallback(async () => {
    try {
      const next = await api.getReview();
      waiting.current = next.waiting;
      setReview(next);
      if (next.waiting) return;
      if (!messageLoaded.current) {
        setMessage(next.message);
        messageLoaded.current = true;
      }
      setViewing((current) => (next.files.some((file) => file.path === current) ? current : (next.files[0]?.path ?? null)));
    } catch (failure) {
      setError(String(failure));
    }
  }, []);

  useEffect(() => {
    void api.beginReview().then(load, (failure) => setError(String(failure)));
    const unlisten = api.onStateChanged(() => {
      if (waiting.current) void load();
    });
    return () => void unlisten.then((stop) => stop());
  }, [load]);

  useEffect(() => {
    if (!viewing) return;
    let current = true;
    setRows(null);
    api.reviewDiff(viewing).then(
      (next) => current && setRows(next),
      (failure) => current && setError(String(failure)),
    );
    return () => {
      current = false;
    };
  }, [viewing]);

  const files = review?.files ?? [];
  const others = review?.others ?? [];
  const ready = review !== null && review.repository && !review.waiting;
  const picked = files.filter((file) => checked.has(file.path)).map((file) => file.path);
  const committable = files.length === 0 ? others.length > 0 : picked.length > 0;

  const check = (paths: string[], on: boolean) =>
    setChecked((current) => {
      const next = new Set(current);
      paths.forEach((path) => (on ? next.add(path) : next.delete(path)));
      return next;
    });

  const commit = async () => {
    setCommitting(true);
    setError(null);
    try {
      const hash = await api.commitReview(message, picked);
      setNotice(`커밋했습니다 (${hash})`);
      setChecked(new Set());
      messageLoaded.current = false;
      await load();
    } catch (failure) {
      setError(String(failure));
    } finally {
      setCommitting(false);
    }
  };

  return (
    <main className="review">
      <header className="review-head">
        <div className="head-text">
          <div className="head-title">기억 저장소 검토</div>
          <div className="head-sub mono" title={review?.root}>
            {review?.root ?? ""}
          </div>
        </div>
        {ready && <span className="review-count">바뀐 문서 {files.length}개</span>}
      </header>

      {error && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <span>{error}</span>
        </div>
      )}

      {review === null ? (
        <div className="review-center">검토할 내용을 불러오는 중입니다.</div>
      ) : !review.repository ? (
        <div className="review-center">{review.root}는 git 저장소가 아니어서 검토할 수 없습니다.</div>
      ) : review.waiting ? (
        <div className="review-center">
          <LoaderCircle size={16} strokeWidth={1.9} className="spin" /> 지금 기록 중인 세션이 끝나면 검토를 시작합니다.
        </div>
      ) : (
        <>
          <div className="review-body">
            <aside className="review-side">
              {files.length === 0 ? (
                <div className="review-note">마지막 커밋 이후 바뀐 기억 문서가 없습니다.</div>
              ) : (
                <>
                  <label className="review-all">
                    <GroupCheck label="전체 선택" paths={files.map((file) => file.path)} checked={checked} onChange={check} />
                    전체 선택
                  </label>
                  <ul className="review-groups" aria-label="바뀐 기억 문서">
                    {byProject(files).map((group) => {
                      const name = fileName(group.folder);
                      return (
                        <li key={group.folder} className="review-group">
                          <div className="review-group-head">
                            <GroupCheck label={`${name} 전체 선택`} paths={group.files.map((file) => file.path)} checked={checked} onChange={check} />
                            <span className="review-group-name">{name}</span>
                            <span className="review-group-count">{group.files.length}</span>
                          </div>
                          <ul className="review-list" aria-label={name}>
                            {group.files.map((file) => (
                              <FileItem
                                key={file.path}
                                file={file}
                                viewing={file.path === viewing}
                                checked={checked.has(file.path)}
                                now={now}
                                onView={() => setViewing(file.path)}
                                onCheck={(on) => check([file.path], on)}
                              />
                            ))}
                          </ul>
                        </li>
                      );
                    })}
                  </ul>
                </>
              )}
              {others.length > 0 && <div className="review-others">함께 커밋되는 다른 파일 {others.length}개 (대화 사본, qmd 설정 등)</div>}
            </aside>
            {viewing && rows ? (
              <DiffView key={viewing} path={viewing} rows={rows} />
            ) : (
              <div className="review-note review-loading">
                {viewing && (
                  <>
                    <LoaderCircle size={15} strokeWidth={1.9} className="spin" /> 변경 내용을 불러오는 중입니다.
                  </>
                )}
              </div>
            )}
          </div>
          <footer className="review-foot">
            <label className="field-label" htmlFor="commit-message">
              커밋 메시지
            </label>
            <textarea id="commit-message" className="input review-message" rows={4} value={message} onChange={(event) => setMessage(event.target.value)} />
            <div className="review-actions">
              <span className="grow">{notice}</span>
              <button type="button" className="btn primary" disabled={committing || !committable || !message.trim()} onClick={() => void commit()}>
                <GitCommitHorizontal size={15} strokeWidth={1.9} />
                {committing ? "커밋 중…" : files.length === 0 ? "커밋" : `선택한 문서 ${picked.length}개 커밋`}
              </button>
            </div>
          </footer>
        </>
      )}
    </main>
  );
}

import { useCallback, useEffect, useRef, useState } from "react";
import { FileText, FileX, LoaderCircle, TriangleAlert } from "lucide-react";

import * as api from "@/ipc/api";
import type { ApplyBatch, Candidate, CandidateKind, CandidateScope, CandidatesSnapshot, NoteResult } from "@/ipc/contract";
import { minutesUntil, providerLabel, relativePhrase } from "@/format";
import { useTheme } from "@/theme";
import { ConfirmCard } from "@/ui/ConfirmCard";
import { Segmented } from "@/ui/Segmented";
import { useNow } from "@/useNow";

const DAY = 86_400;
const KIND: Record<CandidateKind, string> = { decision: "Decision", correction: "Correction", lesson: "Lesson" };
const ACTION: Record<NoteResult["action"], string> = { created: "New note", edited: "Edited", removed: "Removed" };

/** What the right side shows: one candidate, or what became of the approved ones. */
type Viewing = number | "written";

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;

/** How long a candidate has waited, or when it is approved without the user. */
function waitLabel(candidate: Candidate, snapshot: CandidatesSnapshot): { text: string; soon: boolean } {
  if (snapshot.autoApprove) {
    // The days count from the later of two times: when the candidate entered the list, and when
    // automatic approval was turned on. Turning it on approves nothing at once.
    const due = Math.max(candidate.waitingSince, snapshot.autoApproveSince) + snapshot.autoApproveDays * DAY;
    const left = Math.ceil((due - snapshot.now) / DAY);
    if (left <= 0) return { text: "Auto-approves soon", soon: true };
    return { text: `Auto-approves in ${plural(left, "day")}`, soon: left === 1 };
  }
  const days = Math.max(0, Math.floor((snapshot.now - candidate.createdAt) / DAY));
  return { text: days === 0 ? "Found today" : `Waiting ${plural(days, "day")}`, soon: false };
}

const relationLabel = (candidate: Candidate) => (candidate.relation.type === "differs" ? "Differs from a note" : candidate.relation.type === "obsolete" ? "Ends a note" : null);

/** The candidates of one session, in the order they were found. */
function sessions(candidates: Candidate[]): Candidate[][] {
  const groups = new Map<string, Candidate[]>();
  for (const candidate of candidates) {
    const group = groups.get(candidate.jobId);
    if (group) group.push(candidate);
    else groups.set(candidate.jobId, [candidate]);
  }
  return [...groups.values()];
}

function CandidateRow({ candidate, snapshot, viewing, checked, onView, onCheck }: { candidate: Candidate; snapshot: CandidatesSnapshot; viewing: boolean; checked: boolean; onView: () => void; onCheck: (checked: boolean) => void }) {
  const wait = waitLabel(candidate, snapshot);
  const relation = relationLabel(candidate);
  return (
    <li className={`review-item ${viewing ? "active" : ""}`}>
      <input type="checkbox" className="check" aria-label={`Approve: ${candidate.text}`} checked={checked} onChange={(event) => onCheck(event.target.checked)} />
      <button type="button" className="review-item-pick" aria-current={viewing} onClick={onView}>
        <div className="cand-text">{candidate.text}</div>
        <div className="cand-meta">
          <span className={`badge cand-kind ${candidate.kind}`}>{KIND[candidate.kind]}</span>
          <span className="cand-about">
            {candidate.source === "marked" ? "Marked in the session" : "Found afterwards"}
            {relation && <span className={`cand-relation ${candidate.relation.type}`}> · {relation}</span>}
            {candidate.edited && " · Edited"}
          </span>
          <span className={`cand-wait${wait.soon ? " soon" : ""}`}>{wait.text}</span>
        </div>
      </button>
    </li>
  );
}

/** The sentence as the user edits it. It is saved when the field is left, like the fields of the settings. */
function Sentence({ candidate, onSave }: { candidate: Candidate; onSave: (text: string) => void }) {
  const [text, setText] = useState(candidate.text);
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    if (!editing) setText(candidate.text);
  }, [candidate.text, editing]);

  const commit = () => {
    setEditing(false);
    const next = text.trim();
    if (next && next !== candidate.text) onSave(next);
    else setText(candidate.text);
  };

  return <textarea id="candidate-text" className="input cand-sentence" rows={3} value={text} onFocus={() => setEditing(true)} onChange={(event) => setText(event.target.value)} onBlur={commit} />;
}

function Detail({ candidate, snapshot, now, onEdit, onDiscard }: { candidate: Candidate; snapshot: CandidatesSnapshot; now: number; onEdit: (text: string | null, scope: CandidateScope | null) => void; onDiscard: () => void }) {
  const { relation } = candidate;
  const bound = relation.type === "differs" || relation.type === "obsolete";
  const fromAgent = candidate.evidence.every((quote) => quote.speaker === "agent");
  const scopes = [
    { value: "project" as const, label: `This project (${candidate.project})` },
    { value: "common" as const, label: "All projects" },
  ];
  const destination = bound
    ? relation.type === "differs"
      ? "This note is edited. Other notes that say the opposite may be edited too. The result lists every note that changed."
      : "This note is removed, or edited when part of it still holds."
    : `A note about the topic is chosen when it is written, in ${candidate.scope === "common" ? "the notes shared by all projects" : `the notes of ${candidate.project}`}.`;

  return (
    <section className="cleanup-detail" aria-label="Candidate">
      <div className="cleanup-detail-head">
        <div className="head-title cand-detail-title">
          <span className={`badge cand-kind ${candidate.kind}`}>{KIND[candidate.kind]}</span>
          {candidate.project}
        </div>
        <div className="head-sub">
          {providerLabel(candidate.provider)} session · found {relativePhrase(candidate.createdAt, now)} · {candidate.source === "marked" ? "marked by the session while it worked" : "found in the conversation afterwards"}
        </div>
      </div>

      <label className="cleanup-eyebrow cand-label" htmlFor="candidate-text">
        Sentence to remember
      </label>
      <Sentence key={candidate.id} candidate={candidate} onSave={(text) => onEdit(text, null)} />
      <p className="help">Approving puts this into your notes. You can reword it here first.</p>

      {!bound && (
        <>
          <div className="cleanup-eyebrow">Applies to</div>
          <Segmented compact label="Applies to" value={candidate.scope} options={scopes} onChange={(scope) => onEdit(null, scope)} />
        </>
      )}

      <div className="cleanup-eyebrow">Evidence</div>
      {candidate.evidence.map((quote, index) => (
        <blockquote key={`${quote.ref}-${index}`} className="cand-quote">
          <div className="cand-quote-who">
            {quote.speaker === "user" ? "You" : "Agent"}
            {quote.at && ` · ${quote.at.slice(0, 16)}`}
          </div>
          {quote.quote}
        </blockquote>
      ))}
      {fromAgent && <p className="help">These are the agent’s words, not yours. The note says it is the agent’s finding.</p>}

      <div className="cleanup-eyebrow">Existing notes</div>
      {bound ? (
        <>
          <div className="cand-note">
            <button type="button" className="cleanup-note-row open" title={relation.note} aria-label={`Open ${relation.note}`} onClick={() => void api.openNote(relation.note)}>
              <FileText size={14} strokeWidth={1.9} />
              <span className="cleanup-note-main">
                <span className="cleanup-note-name">{relation.title || relation.note}</span>
                <span className="cleanup-note-why mono">{relation.note}</span>
              </span>
            </button>
            <p className="cand-note-label">{relation.type === "differs" ? `${plural(relation.quotes.length, "passage")} of this note that the candidate changes, as written:` : "What this note is about, as written:"}</p>
            {relation.quotes.map((quote) => (
              <blockquote key={quote} className="cand-passage">
                {quote}
              </blockquote>
            ))}
          </div>
        </>
      ) : (
        <p className="cleanup-line">No existing note was found on this. It is added as new content.</p>
      )}

      <div className="cleanup-eyebrow">Where it goes</div>
      <p className="cleanup-line">{destination}</p>

      <div className="cand-actions">
        <span className="cand-wait-note">{waitLabel(candidate, snapshot).text}</span>
        <button type="button" className="pill danger" onClick={onDiscard}>
          Discard this candidate
        </button>
      </div>
    </section>
  );
}

function WrittenBatch({ batch, now }: { batch: ApplyBatch; now: number }) {
  const notes = batch.result?.notes ?? [];
  return (
    <section className="cand-batch" aria-label={`Written for ${batch.project}`}>
      <div className="cleanup-last-head">
        <div>
          <div className="cleanup-line strong">
            {batch.project} · {plural(batch.candidates, "candidate")} into {plural(notes.length, "note")}
          </div>
          <div className="cleanup-standing">
            {relativePhrase(batch.updatedAt, now)}
            {batch.auto && " · approved automatically"}
          </div>
          {batch.status === "index_pending" && batch.lastError && (
            <div className="cleanup-standing cand-late" title={batch.lastError}>
              The notes are written. The search index is not refreshed yet; Lore keeps trying.
            </div>
          )}
        </div>
      </div>
      {batch.result?.summary && <p className="cleanup-summary">{batch.result.summary}</p>}
      {notes.map((note) =>
        note.action === "removed" ? (
          <div key={note.path} className="cleanup-note-row gone" title={note.path}>
            <FileX size={14} strokeWidth={1.9} />
            <NoteLines note={note} />
          </div>
        ) : (
          <button key={note.path} type="button" className="cleanup-note-row open" title={note.path} aria-label={`Open ${note.path}`} onClick={() => void api.openNote(note.path)}>
            <FileText size={14} strokeWidth={1.9} />
            <NoteLines note={note} />
          </button>
        ),
      )}
    </section>
  );
}

function NoteLines({ note }: { note: NoteResult }) {
  return (
    <span className="cleanup-note-main">
      <span className="cand-result-head">
        <span className={`badge cand-action ${note.action}`}>{ACTION[note.action]}</span>
        <span className="cleanup-note-name mono">{note.path}</span>
      </span>
      {note.items.map((item) => (
        <span key={item} className="cleanup-note-why">
          {item}
        </span>
      ))}
    </span>
  );
}

function Written({ snapshot, now, onRetry, onRelease }: { snapshot: CandidatesSnapshot; now: number; onRetry: (id: number) => void; onRelease: (id: number) => void }) {
  const idle = snapshot.applying.length + snapshot.failed.length + snapshot.applied.length === 0;
  return (
    <section className="cleanup-detail" aria-label="Written into the notes">
      <div className="cleanup-detail-head">
        <div className="head-title">Written into your notes</div>
        <div className="head-sub">What became of the candidates you approved. Only notes are shared with your other PCs.</div>
      </div>
      {idle && <p className="cleanup-line">Nothing has been approved yet.</p>}
      {snapshot.applying.map((batch) => (
        <p key={batch.id} className="cleanup-line cand-working">
          <LoaderCircle size={14} strokeWidth={1.9} className="spin" />
          {batch.attempts > 0 && batch.nextAttemptAt > now
            ? `${batch.project}: trying again in about ${minutesUntil(batch.nextAttemptAt, now)} min. ${batch.lastError ?? ""}`
            : `${batch.project}: writing ${plural(batch.candidates, "approved candidate")} into the notes…`}
        </p>
      ))}
      {snapshot.failed.map((batch) => (
        <div key={batch.id} className="banner cand-failed" role="alert">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <div className="cand-failed-main">
            <div>
              {batch.project}: {plural(batch.candidates, "approved candidate")} could not be written. The notes are unchanged.
            </div>
            <div className="cand-failed-why">{batch.lastError}</div>
            <div className="cand-failed-actions">
              <button type="button" className="pill primary" onClick={() => onRetry(batch.id)}>
                Try again
              </button>
              <button type="button" className="pill ghost" onClick={() => onRelease(batch.id)}>
                Put back in the list
              </button>
            </div>
          </div>
        </div>
      ))}
      {snapshot.applied.map((batch) => (
        <WrittenBatch key={batch.id} batch={batch} now={now} />
      ))}
    </section>
  );
}

export function CandidatesApp() {
  useTheme();
  const now = useNow(15_000);
  const [snapshot, setSnapshot] = useState<CandidatesSnapshot | null>(null);
  const [viewing, setViewing] = useState<Viewing | null>(null);
  // The candidates that wait when the window opens are ticked, so only the unticked ones are remembered.
  const [unticked, setUnticked] = useState<ReadonlySet<number>>(new Set());
  const [shownLeftOut, setShownLeftOut] = useState<ReadonlySet<string>>(new Set());
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const discardButton = useRef<HTMLButtonElement>(null);
  // The candidates the window has listed so far. Null until the first load.
  const known = useRef<Set<number> | null>(null);
  const loading = useRef(false);
  const again = useRef(false);

  const load = useCallback(async () => {
    if (loading.current) {
      again.current = true;
      return;
    }
    loading.current = true;
    try {
      const next = await api.getCandidates();
      setSnapshot(next);
      const ids = new Set(next.waiting.map((candidate) => candidate.id));
      // Shown first: the oldest candidate, or what was written when none waits.
      setViewing((current) => (current === "written" || (typeof current === "number" && ids.has(current)) ? current : (next.waiting[0]?.id ?? "written")));
      // A candidate that arrives while the window is open has not been read yet, so it comes unticked:
      // the approve button never takes in something the user did not see when they ticked the rest.
      const seen = known.current;
      const arrived = seen ? [...ids].filter((id) => !seen.has(id)) : [];
      known.current = ids;
      setUnticked((current) => new Set([...current, ...arrived].filter((id) => ids.has(id))));
    } catch (failure) {
      setError(String(failure));
    } finally {
      loading.current = false;
      if (again.current) {
        again.current = false;
        void load();
      }
    }
  }, []);

  useEffect(() => {
    void load();
    const unlisten = api.onStateChanged(() => void load());
    return () => void unlisten.then((stop) => stop());
  }, [load]);

  const act = async (action: () => Promise<unknown>) => {
    setError(null);
    try {
      await action();
      await load();
    } catch (failure) {
      setError(String(failure));
    }
  };

  const waiting = snapshot?.waiting ?? [];
  const picked = waiting.filter((candidate) => !unticked.has(candidate.id)).map((candidate) => candidate.id);
  const viewed = typeof viewing === "number" ? waiting.find((candidate) => candidate.id === viewing) : undefined;
  const writing = (snapshot?.applying.length ?? 0) > 0;
  const written = (snapshot?.applying.length ?? 0) + (snapshot?.failed.length ?? 0) + (snapshot?.applied.length ?? 0);

  const tick = (ids: number[], on: boolean) =>
    setUnticked((current) => {
      const next = new Set(current);
      ids.forEach((id) => (on ? next.delete(id) : next.add(id)));
      return next;
    });

  const approve = () =>
    act(async () => {
      await api.approveCandidates(picked);
      setViewing("written");
    });

  const status = !snapshot
    ? ""
    : writing
      ? "Writing what you approved into the notes…"
      : `Candidates stay on this PC. The ones you leave unticked wait ${snapshot.autoApprove ? `and are approved automatically after ${plural(snapshot.autoApproveDays, "day")}` : "until you approve or discard them"}.`;

  return (
    <main className="review candidates-window">
      <header className="review-head">
        <div className="head-text">
          <div className="head-title">Candidates</div>
          <div className="head-sub">Decisions, corrections and lessons from your sessions. Only what you approve is written into your notes.</div>
        </div>
        {snapshot && <span className="review-count">{plural(waiting.length, "candidate")} waiting</span>}
      </header>

      {error && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <span>{error}</span>
        </div>
      )}

      {snapshot === null ? (
        <div className="review-center">Loading candidates…</div>
      ) : (
        <>
          <div className="review-body">
            <aside className="review-side">
              {waiting.length === 0 ? (
                <div className="review-note">No candidate is waiting. New ones appear when a session ends.</div>
              ) : (
                <>
                  <label className="review-all">
                    <input type="checkbox" className="check" aria-label="Select all" checked={picked.length === waiting.length} onChange={(event) => tick(waiting.map((candidate) => candidate.id), event.target.checked)} />
                    Select all
                    <span className="cand-all-count">
                      {picked.length} of {waiting.length} selected
                    </span>
                  </label>
                  <ul className="review-groups" aria-label="Candidates by session">
                    {sessions(waiting).map((group) => {
                      const first = group[0];
                      const leftOut = snapshot.leftOut.filter((candidate) => candidate.jobId === first.jobId);
                      const open = shownLeftOut.has(first.jobId);
                      const title = `${first.project} · ${providerLabel(first.provider)} · ${relativePhrase(first.createdAt, now)}`;
                      return (
                        <li key={first.jobId} className="review-group">
                          <div className="review-group-head">
                            <span className="review-group-name" title={title}>
                              {title}
                            </span>
                            <span className="review-group-count">{group.length}</span>
                          </div>
                          <ul className="review-groups cleanup-list" aria-label={title}>
                            {group.map((candidate) => (
                              <CandidateRow key={candidate.id} candidate={candidate} snapshot={snapshot} viewing={candidate.id === viewing} checked={!unticked.has(candidate.id)} onView={() => setViewing(candidate.id)} onCheck={(on) => tick([candidate.id], on)} />
                            ))}
                          </ul>
                          {leftOut.length > 0 && (
                            <div className="cand-left-out">
                              {plural(leftOut.length, "statement")} already in your notes {leftOut.length === 1 ? "was" : "were"} left out.{" "}
                              <button type="button" aria-expanded={open} onClick={() => setShownLeftOut((current) => new Set(open ? [...current].filter((id) => id !== first.jobId) : [...current, first.jobId]))}>
                                {open ? "Hide" : "Show"}
                              </button>
                              {open && (
                                <ul>
                                  {leftOut.map((candidate) => (
                                    <li key={candidate.id}>
                                      {candidate.text} <span className="mono">{candidate.relation.note}</span>
                                    </li>
                                  ))}
                                </ul>
                              )}
                            </div>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                </>
              )}
              {written > 0 && waiting.length > 0 && (
                <button type="button" className={`cand-written-link${viewing === "written" ? " active" : ""}`} aria-current={viewing === "written"} onClick={() => setViewing("written")}>
                  {writing && <LoaderCircle size={13} strokeWidth={1.9} className="spin" />}
                  Written into your notes
                </button>
              )}
            </aside>
            {viewed ? (
              <Detail candidate={viewed} snapshot={snapshot} now={now} onEdit={(text, scope) => void act(() => api.editCandidate(viewed.id, text, scope))} onDiscard={() => void act(() => api.discardCandidates([viewed.id]))} />
            ) : (
              <Written snapshot={snapshot} now={now} onRetry={(id) => void act(() => api.retryApply(id))} onRelease={(id) => void act(() => api.releaseApply(id))} />
            )}
          </div>
          <footer className="review-foot">
            <div className="review-actions">
              <span className={`grow plain${writing ? " cand-working" : ""}`}>
                {writing && <LoaderCircle size={14} strokeWidth={1.9} className="spin" />}
                {status}
              </span>
              <button ref={discardButton} type="button" className="btn ghost" disabled={picked.length === 0} aria-expanded={confirming} onClick={() => setConfirming(!confirming)}>
                Discard {picked.length} selected
              </button>
              {confirming && picked.length > 0 && (
                <ConfirmCard
                  anchor={discardButton}
                  label="Discard candidates confirmation"
                  message={`Discard ${plural(picked.length, "selected candidate")}? ${picked.length === 1 ? "It is" : "They are"} not written into your notes and ${picked.length === 1 ? "does" : "do"} not come back.`}
                  action={`Discard ${picked.length}`}
                  onConfirm={() => void act(() => api.discardCandidates(picked))}
                  onClose={() => setConfirming(false)}
                />
              )}
              <button type="button" className="btn primary" disabled={picked.length === 0} onClick={() => void approve()}>
                Approve {picked.length} selected
              </button>
            </div>
          </footer>
        </>
      )}
    </main>
  );
}

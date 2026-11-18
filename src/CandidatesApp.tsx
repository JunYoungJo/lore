import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { ChevronDown, ChevronUp, FileCheck2, FileText, FileX, LoaderCircle, Pencil, Plus, Trash2, TriangleAlert } from "lucide-react";

import * as api from "@/ipc/api";
import type { ApplyBatch, Candidate, CandidateKind, CandidateScope, CandidatesSnapshot, NoteResult } from "@/ipc/contract";
import { minutesUntil, providerLabel, relativePhrase } from "@/format";
import { useTheme } from "@/theme";
import { ConfirmCard } from "@/ui/ConfirmCard";
import { useNow } from "@/useNow";

const DAY = 86_400;
const KIND: Record<CandidateKind, string> = { decision: "Decision", correction: "Correction", lesson: "Lesson" };
const ACTION: Record<NoteResult["action"], string> = { created: "New note", edited: "Edited", removed: "Removed" };

type Tab = "review" | "written";
/** What the user decided for a candidate in this window. Nothing reaches the app until the decisions are applied. */
type Decision = "approve" | "discard" | "later";
type State = Decision | "open";

const STATE: Record<State, string> = { open: "Not decided", approve: "Approved", discard: "Discarded", later: "Later" };
const STATE_LINE: Record<State, string> = { open: "Not decided yet", approve: "Approve", discard: "Discard", later: "Later: keeps waiting" };

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;

/** When a candidate is approved without the user, while automatic approval is on. */
function autoApproval(candidate: Candidate, snapshot: CandidatesSnapshot): { text: string; soon: boolean } | null {
  if (!snapshot.autoApprove) return null;
  // The days count from the later of two times: when the candidate entered the list, and when
  // automatic approval was turned on. Turning it on approves nothing at once.
  const due = Math.max(candidate.waitingSince, snapshot.autoApproveSince) + snapshot.autoApproveDays * DAY;
  const left = Math.ceil((due - snapshot.now) / DAY);
  if (left <= 0) return { text: "Auto-approves soon", soon: true };
  return { text: `Auto-approves in ${plural(left, "day")}`, soon: left === 1 };
}

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

function StateIcon({ state, labelled = false }: { state: State; labelled?: boolean }) {
  const a11y = labelled ? { role: "img", "aria-label": STATE[state] } : { "aria-hidden": true };
  return (
    <svg className={`cand-state ${state}`} width="16" height="16" viewBox="0 0 16 16" {...a11y}>
      {state === "approve" && (
        <>
          <circle cx="8" cy="8" r="7" />
          <path d="M4.8 8.2l2.1 2.1 4.3-4.5" />
        </>
      )}
      {state === "discard" && (
        <>
          <circle cx="8" cy="8" r="7" />
          <path d="M5.5 5.5l5 5M10.5 5.5l-5 5" />
        </>
      )}
      {state === "later" && (
        <>
          <circle cx="8" cy="8" r="6.25" />
          <path d="M8 4.8V8l2.2 1.4" />
        </>
      )}
      {state === "open" && <circle cx="8" cy="8" r="6.25" />}
    </svg>
  );
}

function About({ candidate, snapshot }: { candidate: Candidate; snapshot: CandidatesSnapshot }) {
  const parts: ReactNode[] = [];
  if (candidate.relation.type === "new" && candidate.scope === "common") parts.push("All projects");
  if (candidate.relation.type === "differs") parts.push("Edits a note");
  if (candidate.relation.type === "obsolete")
    parts.push(
      <span key="deletes" className="cand-deletes">
        Deletes a note
      </span>,
    );
  if (candidate.edited) parts.push("Edited");
  const wait = autoApproval(candidate, snapshot);
  if (wait) {
    parts.push(
      <span key="wait" className={`cand-wait${wait.soon ? " soon" : ""}`}>
        {wait.text}
      </span>,
    );
  }
  if (parts.length === 0) return null;
  return <span className="cand-about">{parts.flatMap((part, index) => (index === 0 ? [part] : [" · ", part]))}</span>;
}

function CandidateRow({ candidate, snapshot, state, viewing, onView }: { candidate: Candidate; snapshot: CandidatesSnapshot; state: State; viewing: boolean; onView: () => void }) {
  return (
    <li>
      <button type="button" className={`cand-row ${state}${viewing ? " current" : ""}`} aria-current={viewing} onClick={onView}>
        <StateIcon state={state} labelled />
        <span className="cand-row-main">
          <span className="cand-text">{candidate.text}</span>
          <span className="cand-meta">
            <span className={`badge cand-kind ${candidate.kind}`}>{KIND[candidate.kind]}</span>
            <About candidate={candidate} snapshot={snapshot} />
          </span>
        </span>
      </button>
    </li>
  );
}

/** The sentence, read as text until the user edits it. Edits are saved when the field is left, like the fields of the settings. */
function Sentence({ candidate, editing, onStart, onFinish }: { candidate: Candidate; editing: boolean; onStart: () => void; onFinish: (text: string | null) => void }) {
  const [text, setText] = useState(candidate.text);
  const field = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (!editing) setText(candidate.text);
  }, [candidate.text, editing]);

  const fit = () => {
    const box = field.current;
    if (!box) return;
    box.style.height = "auto";
    box.style.height = `${box.scrollHeight}px`;
  };

  useLayoutEffect(() => {
    if (!editing || !field.current) return;
    fit();
    field.current.focus();
    field.current.setSelectionRange(field.current.value.length, field.current.value.length);
  }, [editing]);

  return (
    <section className="cand-sec first">
      <div className="cand-sec-head">
        Sentence to remember
        {editing ? (
          // The field would lose focus to the button and save on blur first; keeping focus lets the click save once.
          <button type="button" className="cand-edit-link" onMouseDown={(event) => event.preventDefault()} onClick={() => onFinish(text)}>
            Save <kbd>Enter</kbd>
          </button>
        ) : (
          <button type="button" className="cand-edit-link" onClick={onStart}>
            Edit <kbd>E</kbd>
          </button>
        )}
      </div>
      <div className={`cand-sentence-box${editing ? " editing" : ""}`} onClick={editing ? undefined : onStart}>
        {editing ? (
          <textarea
            ref={field}
            className="cand-sentence-edit"
            rows={1}
            aria-label="Sentence to remember"
            value={text}
            onChange={(event) => {
              setText(event.target.value);
              fit();
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                onFinish(text);
              } else if (event.key === "Escape") {
                event.preventDefault();
                onFinish(null);
              }
            }}
            onBlur={() => onFinish(text)}
          />
        ) : (
          <p className="cand-sentence">{candidate.text}</p>
        )}
      </div>
      <p className="cand-help">
        {editing ? (
          <>
            <kbd>Enter</kbd> saves · <kbd>Shift</kbd>+<kbd>Enter</kbd> new line · <kbd>Esc</kbd> cancels · clicking outside saves
          </>
        ) : (
          <>
            Click the sentence or press <kbd>E</kbd> to reword it before approving.
          </>
        )}
      </p>
    </section>
  );
}

function ScopeChoice({ candidate, onChange }: { candidate: Candidate; onChange: (scope: CandidateScope) => void }) {
  const options = [
    { value: "project" as const, key: "1", title: <>This project · <b>{candidate.project}</b></>, detail: `Written into the notes of ${candidate.project}. Only ${candidate.project} sessions read it.` },
    { value: "common" as const, key: "2", title: <>All projects</>, detail: "Written into the shared notes (common). Sessions of every project read it." },
  ];
  return (
    <div className="cand-scope" role="radiogroup" aria-label="Applies to">
      {options.map((option) => (
        <button key={option.value} type="button" role="radio" aria-checked={candidate.scope === option.value} className="cand-scope-opt" onClick={() => candidate.scope !== option.value && onChange(option.value)}>
          <span className="cand-radio" aria-hidden="true" />
          <span className="cand-scope-title">{option.title}</span>
          <kbd>{option.key}</kbd>
          <span className="cand-scope-detail">{option.detail}</span>
        </button>
      ))}
    </div>
  );
}

function Outcome({ candidate, onScope }: { candidate: Candidate; onScope: (scope: CandidateScope) => void }) {
  const { relation } = candidate;
  if (relation.type !== "differs" && relation.type !== "obsolete") {
    return (
      <div className="cand-outcome">
        <div className="cand-outcome-head">
          <Plus size={16} strokeWidth={2.2} />
          Adds this to your notes
        </div>
        <p className="cand-outcome-line">No existing note says this yet. Choose who reads it:</p>
        <ScopeChoice candidate={candidate} onChange={onScope} />
        <p className="cand-outcome-line">The note it goes into is chosen when it is written.</p>
      </div>
    );
  }
  const removing = relation.type === "obsolete";
  return (
    <div className={`cand-outcome${removing ? " delete" : ""}`}>
      <div className="cand-outcome-head">
        {removing ? <Trash2 size={16} strokeWidth={2} /> : <Pencil size={16} strokeWidth={2} />}
        {removing ? "Deletes this note" : "Edits this note"}
      </div>
      <button type="button" className="cleanup-note-row open cand-outcome-note" title={relation.note} aria-label={`Open ${relation.note}`} onClick={() => void api.openNote(relation.note)}>
        <FileText size={14} strokeWidth={1.9} />
        <span className="cleanup-note-main">
          <span className="cleanup-note-name">{relation.title || relation.note}</span>
          <span className="cleanup-note-why mono">{relation.note}</span>
        </span>
      </button>
      <p className="cand-outcome-line">{removing ? "The note says:" : relation.quotes.length === 1 ? "This passage changes:" : `These ${relation.quotes.length} passages change:`}</p>
      {relation.quotes.map((quote) => (
        <blockquote key={quote} className="cand-passage">
          {quote}
        </blockquote>
      ))}
      <p className="cand-outcome-line">{removing ? "If part of the note still holds, that part is kept and only the rest is removed." : "Other notes that say the opposite may be edited too. The result lists every note that changed."}</p>
    </div>
  );
}

interface DetailProps {
  candidate: Candidate;
  snapshot: CandidatesSnapshot;
  now: number;
  state: State;
  position: number;
  count: number;
  editing: boolean;
  onStartEdit: () => void;
  onFinishEdit: (text: string | null) => void;
  onScope: (scope: CandidateScope) => void;
  onDecide: (decision: Decision | null) => void;
  onMove: (step: number) => void;
}

function Detail({ candidate, snapshot, now, state, position, count, editing, onStartEdit, onFinishEdit, onScope, onDecide, onMove }: DetailProps) {
  const scroller = useRef<HTMLDivElement>(null);
  const fromAgent = candidate.evidence.every((quote) => quote.speaker === "agent");
  const wait = autoApproval(candidate, snapshot);

  useEffect(() => {
    if (scroller.current) scroller.current.scrollTop = 0;
  }, [candidate.id]);

  return (
    <section className="cand-detail" aria-label="Candidate">
      <div ref={scroller} className="cand-detail-scroll">
        <div className="cleanup-detail-head">
          <div className="head-title cand-detail-title">
            <span className={`badge cand-kind ${candidate.kind}`}>{KIND[candidate.kind]}</span>
            {candidate.project}
          </div>
          <div className="head-sub">
            {providerLabel(candidate.provider)} session · found {relativePhrase(candidate.createdAt, now)} · {candidate.source === "marked" ? "marked by the session while it worked" : "found in the conversation afterwards"}
            {candidate.edited && " · edited by you"}
            {wait && ` · ${wait.text}`}
          </div>
        </div>

        <Sentence key={candidate.id} candidate={candidate} editing={editing} onStart={onStartEdit} onFinish={onFinishEdit} />

        <section className="cand-sec">
          <div className="cand-sec-head">
            Evidence <span className="cand-sec-sub">what was said in the session</span>
          </div>
          {candidate.evidence.map((quote, index) => (
            <blockquote key={`${quote.ref}-${index}`} className="cand-quote">
              <div className="cand-quote-who">
                {quote.speaker === "user" ? "You" : "Agent"}
                {quote.at && ` · ${quote.at.slice(0, 16)}`}
              </div>
              {quote.quote}
            </blockquote>
          ))}
          {fromAgent && <p className="cand-help">These are the agent’s words, not yours. The note says it is the agent’s finding.</p>}
        </section>

        <section className="cand-sec">
          <div className="cand-sec-head">What approving does</div>
          <Outcome candidate={candidate} onScope={onScope} />
        </section>
      </div>

      <div className="cand-decide">
        <span className="cand-nav">
          <button type="button" className="cand-icon-btn" aria-label="Previous candidate" disabled={position <= 1} onClick={() => onMove(-1)}>
            <ChevronUp size={14} strokeWidth={2} />
          </button>
          <button type="button" className="cand-icon-btn" aria-label="Next candidate" disabled={position >= count} onClick={() => onMove(1)}>
            <ChevronDown size={14} strokeWidth={2} />
          </button>
          {position} of {count}
        </span>
        <span className={`cand-state-line ${state}`}>
          <StateIcon state={state} />
          <b>{STATE_LINE[state]}</b>
          {state !== "open" && (
            <button type="button" className="cand-undo" onClick={() => onDecide(null)}>
              Undo <kbd>U</kbd>
            </button>
          )}
        </span>
        <button type="button" className={`cand-btn discard${state === "discard" ? " on" : ""}`} onClick={() => onDecide("discard")}>
          Discard <kbd>D</kbd>
        </button>
        <button type="button" className={`cand-btn later${state === "later" ? " on" : ""}`} onClick={() => onDecide("later")}>
          Later <kbd>L</kbd>
        </button>
        <button type="button" className={`cand-btn approve${state === "approve" ? " on" : ""}`} onClick={() => onDecide("approve")}>
          Approve and next <kbd>A</kbd>
        </button>
      </div>
    </section>
  );
}

const clock = (seconds: number) => new Intl.DateTimeFormat("en-US", { hour: "2-digit", minute: "2-digit", hour12: false }).format(new Date(seconds * 1000));

/** The day a batch is listed under: today, yesterday, or its date. */
function dayLabel(seconds: number, now: number): string {
  const day = (value: number) => new Date(value * 1000).toDateString();
  if (day(seconds) === day(now)) return "Today";
  if (day(seconds) === day(now - DAY)) return "Yesterday";
  return new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric" }).format(new Date(seconds * 1000));
}

interface Listed {
  batch: ApplyBatch;
  state: "writing" | "failed" | "written";
}

/** Every batch the window knows of, newest first. */
function listed(snapshot: CandidatesSnapshot): Listed[] {
  return [
    ...snapshot.applying.map((batch) => ({ batch, state: "writing" as const })),
    ...snapshot.failed.map((batch) => ({ batch, state: "failed" as const })),
    ...snapshot.applied.map((batch) => ({ batch, state: "written" as const })),
  ].sort((a, b) => b.batch.updatedAt - a.batch.updatedAt || b.batch.id - a.batch.id);
}

const writingLine = (batch: ApplyBatch, now: number) =>
  batch.attempts > 0 && batch.nextAttemptAt > now ? `Trying again in about ${minutesUntil(batch.nextAttemptAt, now)} min. ${batch.lastError ?? ""}`.trim() : `Writing ${plural(batch.candidates, "approved candidate")} into the notes…`;

function BatchRow({ item, now, current, onView }: { item: Listed; now: number; current: boolean; onView: () => void }) {
  const { batch, state } = item;
  return (
    <li>
      <button type="button" className={`cand-row${current ? " current" : ""}`} aria-current={current} onClick={onView}>
        <span className={`cand-batch-icon ${state}`} aria-hidden="true">
          {state === "writing" ? <LoaderCircle size={15} strokeWidth={1.9} className="spin" /> : state === "failed" ? <TriangleAlert size={15} strokeWidth={1.9} /> : <FileCheck2 size={15} strokeWidth={1.9} />}
        </span>
        <span className="cand-row-main">
          <span className="cand-batch-title">
            {batch.project} · {clock(batch.updatedAt)}
          </span>
          <span className="cand-meta">
            {state === "failed" ? (
              <span>
                <span className="cand-deletes">Could not be written</span> · {plural(batch.candidates, "candidate")}
              </span>
            ) : state === "writing" ? (
              <span className="cand-about">{writingLine(batch, now)}</span>
            ) : (
              <span className="cand-about">
                {plural(batch.candidates, "candidate")} into {plural(batch.result?.notes.length ?? 0, "note")}
                {batch.auto && " · approved automatically"}
              </span>
            )}
          </span>
        </span>
      </button>
    </li>
  );
}

function NoteResults({ notes }: { notes: NoteResult[] }) {
  return (
    <div className="cand-results">
      {notes.map((note) => (
        <div key={note.path} className="cand-result">
          <div className="cand-result-head">
            <span className={`cand-result-action ${note.action}`}>{ACTION[note.action]}</span>
            {note.action === "removed" ? (
              <span className="cand-result-path gone" title={note.path}>
                <FileX size={13} strokeWidth={1.9} />
                {note.path}
              </span>
            ) : (
              <button type="button" className="cand-result-path" title={note.path} aria-label={`Open ${note.path}`} onClick={() => void api.openNote(note.path)}>
                <FileText size={13} strokeWidth={1.9} />
                {note.path}
              </button>
            )}
          </div>
          <ul className="cand-result-items">
            {note.items.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}

function BatchDetail({ item, now, onRetry, onRelease }: { item: Listed; now: number; onRetry: () => void; onRelease: () => void }) {
  const { batch, state } = item;
  const notes = batch.result?.notes ?? [];
  return (
    <section className="cand-detail" aria-label={`Written for ${batch.project}`}>
      <div className="cand-detail-scroll">
        <div className="cleanup-detail-head">
          <div className="head-title">{batch.project}</div>
          <div className="head-sub">
            {dayLabel(batch.updatedAt, now)} {clock(batch.updatedAt)}
            {state === "written" && ` · ${plural(batch.candidates, "candidate")} into ${plural(notes.length, "note")} · ${batch.auto ? "approved automatically" : "approved by you"}`}
          </div>
        </div>
        {state === "failed" && (
          <div className="cand-failed-box" role="alert">
            <div className="cand-failed-head">{plural(batch.candidates, "approved candidate")} could not be written. The notes are unchanged.</div>
            {batch.lastError && <div className="cand-failed-why">{batch.lastError}</div>}
            <div className="cand-failed-actions">
              <button type="button" className="btn primary" onClick={onRetry}>
                Try again
              </button>
              <button type="button" className="btn ghost" onClick={onRelease}>
                Put back in the list
              </button>
            </div>
          </div>
        )}
        {state === "writing" && (
          <p className="cleanup-line cand-working">
            <LoaderCircle size={14} strokeWidth={1.9} className="spin" />
            {writingLine(batch, now)}
          </p>
        )}
        {state === "written" && (
          <>
            {batch.status === "index_pending" && batch.lastError && (
              <p className="cleanup-standing cand-late" title={batch.lastError}>
                The notes are written. The search index is not refreshed yet; Lore keeps trying.
              </p>
            )}
            {batch.result?.summary && (
              <section className="cand-sec first">
                <div className="cand-sec-head">Summary</div>
                <p className="cleanup-summary">{batch.result.summary}</p>
              </section>
            )}
            <section className={`cand-sec${batch.result?.summary ? "" : " first"}`}>
              <div className="cand-sec-head">
                Notes <span className="cand-sec-sub">which sentence went into which note</span>
              </div>
              <NoteResults notes={notes} />
            </section>
          </>
        )}
      </div>
    </section>
  );
}

function WrittenView({ snapshot, now, selected, onSelect, onRetry, onRelease }: { snapshot: CandidatesSnapshot; now: number; selected: number | null; onSelect: (id: number) => void; onRetry: (id: number) => void; onRelease: (id: number) => void }) {
  const items = listed(snapshot);
  const shown = items.find((item) => item.batch.id === selected) ?? items[0];
  const days = [...new Set(items.map((item) => dayLabel(item.batch.updatedAt, now)))];
  return (
    <div className="review-body">
      <aside className="review-side">
        {items.length === 0 ? (
          <div className="review-note">Nothing has been written yet. What you approve appears here once Lore writes it.</div>
        ) : (
          <ul className="review-groups" aria-label="Written batches">
            {days.map((day) => (
              <li key={day} className="review-group">
                <div className="review-group-head">
                  <span className="review-group-name">{day}</span>
                </div>
                <ul className="cand-rows" aria-label={day}>
                  {items
                    .filter((item) => dayLabel(item.batch.updatedAt, now) === day)
                    .map((item) => (
                      <BatchRow key={item.batch.id} item={item} now={now} current={item === shown} onView={() => onSelect(item.batch.id)} />
                    ))}
                </ul>
              </li>
            ))}
          </ul>
        )}
      </aside>
      {shown ? <BatchDetail item={shown} now={now} onRetry={() => onRetry(shown.batch.id)} onRelease={() => onRelease(shown.batch.id)} /> : <div className="review-center">Nothing has been written yet.</div>}
    </div>
  );
}

export function CandidatesApp() {
  useTheme();
  const now = useNow(15_000);
  const [snapshot, setSnapshot] = useState<CandidatesSnapshot | null>(null);
  const [tab, setTab] = useState<Tab>("review");
  const [viewing, setViewing] = useState<number | null>(null);
  const [selectedBatch, setSelectedBatch] = useState<number | null>(null);
  const [confirmingClear, setConfirmingClear] = useState(false);
  const clearButton = useRef<HTMLButtonElement>(null);
  const [decisions, setDecisions] = useState<ReadonlyMap<number, Decision>>(new Map());
  // A candidate that arrives while the window is open has not been read yet, so approving the
  // undecided ones leaves it out until the user has looked at it.
  const [unseen, setUnseen] = useState<ReadonlySet<number>>(new Set());
  const [editing, setEditing] = useState(false);
  // Enter and the blur that follows when the field goes away both finish the edit; only the first one counts.
  const editingNow = useRef(false);
  const [shownLeftOut, setShownLeftOut] = useState<ReadonlySet<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  // The candidates the window has listed so far. Null until the first load.
  const known = useRef<Set<number> | null>(null);
  // The order of the list as last loaded, so a candidate that leaves it is followed by the one after it.
  const order = useRef<number[]>([]);
  const loading = useRef(false);
  const again = useRef(false);
  const keys = useRef<(event: KeyboardEvent) => void>(() => {});

  const load = useCallback(async () => {
    if (loading.current) {
      again.current = true;
      return;
    }
    loading.current = true;
    try {
      const next = await api.getCandidates();
      setSnapshot(next);
      const ids = next.waiting.map((candidate) => candidate.id);
      const present = new Set(ids);
      const previous = order.current;
      order.current = ids;
      setDecisions((current) => new Map([...current].filter(([id]) => present.has(id))));
      setViewing((current) => {
        if (current !== null && present.has(current)) return current;
        if (current !== null) {
          const after = previous.slice(previous.indexOf(current) + 1).find((id) => present.has(id));
          if (after !== undefined) return after;
        }
        return ids[0] ?? null;
      });
      const seen = known.current;
      const arrived = seen ? ids.filter((id) => !seen.has(id)) : [];
      known.current = present;
      setUnseen((current) => new Set([...current, ...arrived].filter((id) => present.has(id))));
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

  useEffect(() => {
    const focus = async () => {
      const batch = await api.takeCandidatesFocus();
      if (batch === null) return;
      setSelectedBatch(batch);
      setTab("written");
    };
    void focus();
    const unlisten = api.onCandidatesFocus(() => void focus());
    return () => void unlisten.then((stop) => stop());
  }, []);

  useEffect(() => {
    if (viewing !== null && unseen.has(viewing)) setUnseen((current) => new Set([...current].filter((id) => id !== viewing)));
  }, [viewing, unseen]);

  useEffect(() => {
    const listener = (event: KeyboardEvent) => keys.current(event);
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, []);

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
  const viewed = viewing === null ? undefined : waiting.find((candidate) => candidate.id === viewing);
  const stateOf = (id: number): State => decisions.get(id) ?? "open";
  const writing = (snapshot?.applying.length ?? 0) > 0;
  const failedCount = snapshot?.failed.length ?? 0;
  const pendingBatches = (snapshot?.applying.length ?? 0) + failedCount;
  const clearable = snapshot?.applied.filter((batch) => batch.status === "done").length ?? 0;
  const tally = { approve: 0, discard: 0, later: 0, open: 0 };
  waiting.forEach((candidate) => tally[stateOf(candidate.id)]++);
  const decided = tally.approve + tally.discard + tally.later;
  const undecided = waiting.filter((candidate) => !decisions.has(candidate.id) && !unseen.has(candidate.id));

  const edit = (on: boolean) => {
    editingNow.current = on;
    setEditing(on);
  };

  const view = (id: number | null) => {
    edit(false);
    setViewing(id);
  };

  /** The first candidate after `from` that is not decided yet, going round the list; else simply the next one. */
  const nextOpen = (from: number, decided: ReadonlyMap<number, Decision>, ids = waiting.map((candidate) => candidate.id)) => {
    const start = ids.indexOf(from);
    for (let step = 1; step < ids.length; step++) {
      const id = ids[(start + step) % ids.length];
      if (!decided.has(id)) return id;
    }
    return ids[Math.min(start + 1, ids.length - 1)];
  };

  const decide = (decision: Decision | null) => {
    if (!viewed) return;
    const next = new Map(decisions);
    if (decision) next.set(viewed.id, decision);
    else next.delete(viewed.id);
    setDecisions(next);
    if (decision) view(nextOpen(viewed.id, next) ?? viewed.id);
  };

  const move = (step: number) => {
    if (!viewed) return;
    const index = waiting.indexOf(viewed);
    const target = waiting[Math.max(0, Math.min(waiting.length - 1, index + step))];
    if (target) view(target.id);
  };

  const changeScope = (scope: CandidateScope) => {
    if (!viewed || viewed.relation.type === "differs" || viewed.relation.type === "obsolete" || viewed.scope === scope) return;
    void act(() => api.editCandidate(viewed.id, null, scope));
  };

  const finishEdit = (text: string | null) => {
    if (!editingNow.current || !viewed) return;
    edit(false);
    if (text === null) return;
    const next = text
      .split("\n")
      .map((line) => line.trimEnd())
      .join("\n")
      .trim();
    if (next && next !== viewed.text) void act(() => api.editCandidate(viewed.id, next, null));
  };

  const apply = () =>
    act(async () => {
      const approve = waiting.filter((candidate) => decisions.get(candidate.id) === "approve").map((candidate) => candidate.id);
      const discard = waiting.filter((candidate) => decisions.get(candidate.id) === "discard").map((candidate) => candidate.id);
      const sent = new Set([...approve, ...discard]);
      if (approve.length > 0) await api.approveCandidates(approve);
      if (discard.length > 0) await api.discardCandidates(discard);
      const rest = waiting.map((candidate) => candidate.id).filter((id) => !sent.has(id));
      const kept = new Map([...decisions].filter(([id]) => !sent.has(id)));
      setDecisions(kept);
      if (viewing !== null && rest.includes(viewing)) return;
      const after = viewing === null ? [] : waiting.map((candidate) => candidate.id).slice(waiting.findIndex((candidate) => candidate.id === viewing) + 1);
      const target = after.find((id) => rest.includes(id) && !kept.has(id)) ?? rest.find((id) => !kept.has(id)) ?? rest[0];
      view(target ?? null);
      if (target === undefined && approve.length > 0) {
        setSelectedBatch(null);
        setTab("written");
      }
    });

  keys.current = (event) => {
    if (event.metaKey || event.ctrlKey || event.altKey || event.defaultPrevented || tab !== "review" || !viewed) return;
    if (event.target instanceof Element && event.target.closest("textarea, input, select")) return;
    const run = (
      {
        a: () => decide("approve"),
        d: () => decide("discard"),
        l: () => decide("later"),
        u: () => decide(null),
        j: () => move(1),
        arrowdown: () => move(1),
        k: () => move(-1),
        arrowup: () => move(-1),
        "1": () => changeScope("project"),
        "2": () => changeScope("common"),
        e: () => edit(true),
      } as Record<string, () => void>
    )[event.key.toLowerCase()];
    if (!run) return;
    event.preventDefault();
    run();
  };

  const applying = [tally.approve > 0 && `approve ${tally.approve}`, tally.discard > 0 && `discard ${tally.discard}`].filter(Boolean).join(", ");
  const status = !snapshot
    ? ""
    : writing
      ? "Writing what you approved into the notes…"
      : `Nothing is written until you apply. Later and undecided candidates keep waiting${snapshot.autoApprove ? ` and are approved automatically after ${plural(snapshot.autoApproveDays, "day")}` : ""}.`;

  return (
    <main className="review candidates-window">
      <header className="review-head">
        <div className="head-text">
          <div className="head-title">Candidates</div>
          <div className="head-sub">Decisions, corrections and lessons from your sessions. Only what you approve is written into your notes.</div>
        </div>
        {snapshot && tab === "review" && (
          <div className="cand-progress">
            <span className="review-count">
              {plural(waiting.length, "candidate")} waiting · {decided} decided
            </span>
            <span className="cand-bar" aria-hidden="true">
              {(["approve", "discard", "later"] as const).map((state) => (
                <span key={state} className={state} style={{ width: `${waiting.length ? (tally[state] / waiting.length) * 100 : 0}%` }} />
              ))}
            </span>
          </div>
        )}
      </header>

      {snapshot && (
        <div className="cand-tabs" role="tablist" aria-label="Candidates views">
          <button type="button" role="tab" aria-selected={tab === "review"} className="cand-tab" onClick={() => setTab("review")}>
            To review <span className="cand-tab-count">{waiting.length}</span>
          </button>
          <button type="button" role="tab" aria-selected={tab === "written"} className="cand-tab" onClick={() => setTab("written")}>
            Written {pendingBatches > 0 && <span className={`cand-tab-count${failedCount > 0 ? " alert" : ""}`}>{pendingBatches}</span>}
          </button>
        </div>
      )}

      {error && (
        <div className="banner" role="alert">
          <TriangleAlert size={15} strokeWidth={1.9} style={{ flex: "none", marginTop: 1 }} />
          <span>{error}</span>
        </div>
      )}

      {snapshot === null ? (
        <div className="review-center">Loading candidates…</div>
      ) : tab === "written" ? (
        <>
          <WrittenView snapshot={snapshot} now={now} selected={selectedBatch} onSelect={setSelectedBatch} onRetry={(id) => void act(() => api.retryApply(id))} onRelease={(id) => void act(() => api.releaseApply(id))} />
          <footer className="review-foot">
            <div className="review-actions">
              <span className="grow plain">Written batches stay here for 30 days. The notes keep what was written; only the notes are shared with your other PCs.</span>
              <button ref={clearButton} type="button" className="btn ghost" disabled={clearable === 0} aria-expanded={confirmingClear} onClick={() => setConfirmingClear(!confirmingClear)}>
                Clear
              </button>
              {confirmingClear && clearable > 0 && (
                <ConfirmCard
                  anchor={clearButton}
                  label="Clear written batches confirmation"
                  message={`Remove ${plural(clearable, "written batch", "written batches")} from this list? The notes stay as they are. Batches being written or that failed are kept.`}
                  action="Clear"
                  onConfirm={() => void act(() => api.clearWritten())}
                  onClose={() => setConfirmingClear(false)}
                />
              )}
            </div>
          </footer>
        </>
      ) : (
        <>
          <div className="review-body">
            <aside className="review-side">
              {waiting.length === 0 ? (
                <div className="review-note">No candidate is waiting. New ones appear when a session ends.</div>
              ) : (
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
                          <span className="review-group-count">
                            {group.filter((candidate) => decisions.has(candidate.id)).length}/{group.length}
                          </span>
                        </div>
                        <ul className="cand-rows" aria-label={title}>
                          {group.map((candidate) => (
                            <CandidateRow key={candidate.id} candidate={candidate} snapshot={snapshot} state={stateOf(candidate.id)} viewing={candidate.id === viewing} onView={() => view(candidate.id)} />
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
              )}
            </aside>
            {viewed ? (
              <Detail
                candidate={viewed}
                snapshot={snapshot}
                now={now}
                state={stateOf(viewed.id)}
                position={waiting.indexOf(viewed) + 1}
                count={waiting.length}
                editing={editing}
                onStartEdit={() => edit(true)}
                onFinishEdit={finishEdit}
                onScope={changeScope}
                onDecide={decide}
                onMove={move}
              />
            ) : (
              <div className="review-center">Nothing left to review.</div>
            )}
          </div>
          <footer className="review-foot">
            <div className="review-actions cand-foot">
              <span className="cand-tally" aria-label="Decisions">
                {(["approve", "discard", "later", "open"] as const).map((state) => (
                  <span key={state}>
                    <StateIcon state={state} />
                    <b>{tally[state]}</b> {state === "open" ? "not decided" : state}
                  </span>
                ))}
              </span>
              <span className={`grow plain${writing ? " cand-working" : ""}`}>
                {writing && <LoaderCircle size={14} strokeWidth={1.9} className="spin" />}
                {status}
              </span>
              <button type="button" className="btn ghost" disabled={undecided.length === 0} onClick={() => setDecisions(new Map([...decisions, ...undecided.map((candidate) => [candidate.id, "approve"] as const)]))}>
                Approve {plural(undecided.length, "undecided candidate")}
              </button>
              <button type="button" className="btn primary" disabled={applying === ""} onClick={() => void apply()}>
                {applying ? `Apply: ${applying}` : "Apply"}
              </button>
            </div>
          </footer>
        </>
      )}
    </main>
  );
}

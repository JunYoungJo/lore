import { useMemo, useState } from "react";

import type { DiffRow, RowKind, Segment } from "@/ipc/contract";

// A memory note keeps one paragraph per line, so a run of unchanged lines fills the screen much sooner than code does.
const FOLD_OVER = 6;
const KEEP_HEAD = 2;
const KEEP_TAIL = 2;

type Item = { type: "row"; row: DiffRow; key: number } | { type: "fold"; start: number; hidden: number };

const ROW_LABEL: Record<RowKind, string> = { same: "Unchanged", changed: "Changed", removed: "Deleted", added: "Added" };

function Text({ segments }: { segments: Segment[] }) {
  return (
    <>
      {segments.map((segment, index) =>
        segment.kind === "removed" ? <del key={index}>{segment.text}</del> : segment.kind === "added" ? <ins key={index}>{segment.text}</ins> : <span key={index}>{segment.text}</span>,
      )}
    </>
  );
}

function Cell({ no, segments, side }: { no: number | null; segments: Segment[] | null; side: "left" | "right" }) {
  if (segments === null) return <div role="cell" className={`dcell ${side} pad`} />;
  return (
    <div role="cell" className={`dcell ${side}`}>
      <span className="ln">{no}</span>
      <span className="tx">
        <Text segments={segments} />
      </span>
    </div>
  );
}

export function DiffView({ path, rows }: { path: string; rows: DiffRow[] }) {
  const [opened, setOpened] = useState<Set<number>>(new Set());
  const [showAll, setShowAll] = useState(false);

  const stats = useMemo(() => {
    let removed = 0;
    let added = 0;
    for (const row of rows) {
      if (row.kind === "removed" || row.kind === "changed") removed += 1;
      if (row.kind === "added" || row.kind === "changed") added += 1;
    }
    return { removed, added };
  }, [rows]);

  const items = useMemo(() => {
    const result: Item[] = [];
    let start = 0;
    while (start < rows.length) {
      let end = start;
      while (end < rows.length && rows[end].kind === rows[start].kind) end += 1;
      const fold = rows[start].kind === "same" && !showAll && !opened.has(start) && end - start > FOLD_OVER;
      for (let index = start; index < end; index += 1) {
        if (fold && index === start + KEEP_HEAD) {
          result.push({ type: "fold", start, hidden: end - start - KEEP_HEAD - KEEP_TAIL });
          index = end - KEEP_TAIL - 1;
          continue;
        }
        result.push({ type: "row", row: rows[index], key: index });
      }
      start = end;
    }
    return result;
  }, [rows, opened, showAll]);

  const anyFolded = items.some((item) => item.type === "fold");

  return (
    <section className="diff" aria-label={`Changes in ${path}`}>
      <div className="diff-bar">
        <span className="diff-path mono" title={path}>
          {path}
        </span>
        <span className="diff-stat del">−{stats.removed} lines</span>
        <span className="diff-stat add">+{stats.added} lines</span>
        <span className="legend">
          <i className="sw del" /> Removed <i className="sw add" /> Added
        </span>
        {(anyFolded || showAll) && (
          <button type="button" className="link-btn diff-toggle" onClick={() => setShowAll(!showAll)}>
            {showAll ? "Fold unchanged lines" : "Expand all"}
          </button>
        )}
      </div>
      <div className="diff-head">
        <div>Last commit</div>
        <div>Current</div>
      </div>
      <div className="diff-rows" role="table" aria-label="Side-by-side diff">
        {items.map((item) =>
          item.type === "fold" ? (
            <button key={`fold-${item.start}`} type="button" className="diff-fold" aria-label={`Expand ${item.hidden} unchanged lines`} onClick={() => setOpened(new Set(opened).add(item.start))}>
              ⋯ Expand {item.hidden} unchanged lines ⋯
            </button>
          ) : (
            <div key={item.key} role="row" aria-label={`Line ${item.row.rightNo ?? item.row.leftNo}: ${ROW_LABEL[item.row.kind]}`} className={`drow ${item.row.kind}`}>
              <Cell no={item.row.leftNo} segments={item.row.left} side="left" />
              <Cell no={item.row.rightNo} segments={item.row.right} side="right" />
            </div>
          ),
        )}
      </div>
    </section>
  );
}

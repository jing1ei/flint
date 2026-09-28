/**
 * The queue. A hairline-separated list with one quiet header line above it: the per-category bulk
 * pick (only when it would actually save clicks) and "Clear all".
 *
 * The count lives in the top strip now, so nothing here repeats it.
 */
import { useEffect, useMemo, useRef } from "react";
import { categoryPlural } from "../lib/format";
import type { CategoryId } from "../lib/types";
import { allRows, ownsRunningBatch, useStore, type Row } from "../state/store";
import { FileRow } from "./FileRow";
import { TargetPicker } from "./TargetPicker";

interface Group {
  category: CategoryId;
  suggested: string[];
  target: string;
  count: number;
}

export function FileList(): React.JSX.Element {
  const files = useStore((s) => s.files);
  const order = useStore((s) => s.order);
  /*
   * Locked only while *this* window's rows are in the batch. A conversion inherited from a previous
   * page load has no rows here at all, so emptying the queue or re-picking a target takes nothing
   * away from it, and a control that cannot do any harm has no business looking dead.
   */
  const locked = useStore(ownsRunningBatch);
  const clearAll = useStore((s) => s.clearAll);
  const setCategoryTarget = useStore((s) => s.setCategoryTarget);

  const rows = useMemo(() => allRows({ files, order }), [files, order]);

  // A bulk control only earns its place when it would actually save clicks: >1 file of a category.
  const groups = useMemo<Group[]>(() => {
    const buckets = new Map<CategoryId, Row[]>();
    for (const row of rows) {
      if (!row.info.supported || row.info.category === null) continue;
      const bucket = buckets.get(row.info.category);
      if (bucket === undefined) buckets.set(row.info.category, [row]);
      else bucket.push(row);
    }
    const out: Group[] = [];
    for (const [category, members] of buckets) {
      const first = members[0];
      if (members.length < 2 || first === undefined) continue;
      const shared = members.every((r) => r.target === first.target) ? first.target : "";
      out.push({
        category,
        suggested: first.info.suggested_targets,
        target: shared,
        count: members.length,
      });
    }
    return out;
  }, [rows]);

  /*
   * Where the keyboard goes when a row is removed.
   *
   * Removing the row that has focus destroys the focused element, and the browser's answer to that
   * is `<body>`: the ring disappeared and the next Tab started again from the top of the window, so
   * pruning a twenty-file queue by keyboard meant twenty trips back through the header. The row that
   * took its place is the obvious place to stand, and `removeFile` has already moved the selection
   * there, so ⌫ can simply be pressed again.
   *
   * Only when focus was actually lost: a removal that left the keyboard somewhere real (the × of
   * another row, a control in the header) must not have it snatched away.
   */
  const listRef = useRef<HTMLUListElement>(null);
  const previous = useRef(order);
  useEffect(() => {
    const before = previous.current;
    previous.current = order;
    const list = listRef.current;
    if (order.length === 0 || order.length >= before.length || list === null) return;
    if (document.activeElement !== document.body) return;
    const still = new Set(order);
    const index = before.findIndex((id) => !still.has(id));
    if (index === -1) return;
    const node = list.children[Math.min(index, order.length - 1)];
    if (node instanceof HTMLElement) node.focus();
  }, [order]);

  return (
    <section className="filelist" aria-label="Files to convert">
      <div className="filelist__header">
        <div className="filelist__bulk">
          {groups.map((group) => (
            <label key={group.category} className="filelist__bulkitem">
              <span className="filelist__bulklabel">
                All {group.count} {categoryPlural(group.category)}
              </span>
              <TargetPicker
                category={group.category}
                suggested={group.suggested}
                value={group.target}
                disabled={locked}
                // The same words the label beside it uses: "All 3 videos" / "Convert all 3 videos to".
                label={`Convert all ${group.count} ${categoryPlural(group.category)} to`}
                onChange={(target) => setCategoryTarget(group.category, target)}
              />
            </label>
          ))}
        </div>
        <button type="button" className="microlink" onClick={clearAll} disabled={locked}>
          Clear all
        </button>
      </div>

      <ul className="filelist__rows" ref={listRef}>
        {order.map((id, index) => (
          <FileRow key={id} id={id} index={index} />
        ))}
      </ul>
    </section>
  );
}

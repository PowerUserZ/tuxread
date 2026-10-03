// The folder's entries (spec §5.9): a grid that renders only the rows in view, so folders with
// tens of thousands of entries scroll smoothly.
import { type KeyboardEvent, type MouseEvent, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { EntryView } from "./api";
import { formatMode, formatSize, formatTime } from "./format";
import { type Key, useI18n } from "./i18n";
import { FileIcon, FolderIcon, LinkIcon, SpecialIcon } from "./icons";
import type { ClickMode, Selection, Sort, SortKey } from "./listing";

export const ROW = 28;
/** The column header's height; it stays at the top of the scrolling area. */
const HEAD = 32;

const columns: { key: SortKey; label: Key; className: string }[] = [
  { key: "name", label: "colName", className: "c-name" },
  { key: "size", label: "colSize", className: "c-size" },
  { key: "mtime", label: "colModified", className: "c-time" },
  { key: "mode", label: "colPermissions", className: "c-mode" },
  { key: "owner", label: "colOwner", className: "c-owner" },
];

const kindIcon = {
  dir: <FolderIcon />,
  file: <FileIcon />,
  symlink: <LinkIcon />,
  other: <SpecialIcon />,
};

type Props = {
  entries: EntryView[];
  selection: Selection;
  sort: Sort;
  label: string;
  empty: string | null;
  onSort: (key: SortKey) => void;
  onClick: (id: number, mode: ClickMode) => void;
  onActivate: (entry: EntryView) => void;
  onKeyDown: (e: KeyboardEvent<HTMLDivElement>, pageRows: number) => void;
};

export function FileList(props: Props) {
  const { t, lang } = useI18n();
  const { entries, selection, sort } = props;
  const body = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(400);

  useLayoutEffect(() => {
    const el = body.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setHeight(el.clientHeight));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Keep the focused row in view (below the sticky header) as the keyboard moves it.
  useEffect(() => {
    const el = body.current;
    const index = entries.findIndex((e) => e.id === selection.focus);
    if (!el || index === -1) return;
    const top = HEAD + index * ROW;
    if (top < el.scrollTop + HEAD) el.scrollTop = top - HEAD;
    else if (top + ROW > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW - el.clientHeight;
  }, [selection.focus, entries]);

  const first = Math.max(0, Math.floor(scrollTop / ROW) - 8);
  const last = Math.min(entries.length, Math.ceil((scrollTop + height) / ROW) + 8);
  const pageRows = Math.max(1, Math.floor((height - HEAD) / ROW) - 1);
  const modeOf = (e: MouseEvent): ClickMode => (e.shiftKey ? "range" : e.ctrlKey ? "toggle" : "replace");

  return (
    <div
      className="filelist"
      role="grid"
      aria-label={props.label}
      aria-multiselectable="true"
      aria-rowcount={entries.length + 1}
      aria-activedescendant={selection.focus === null ? undefined : `row-${selection.focus}`}
      tabIndex={0}
      // Keys pressed on a header button are the button's (Enter sorts), not the list's.
      onKeyDown={(e) => e.target === e.currentTarget && props.onKeyDown(e, pageRows)}
    >
      <div className="body" ref={body} onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}>
      <div className="row head" role="row" aria-rowindex={1}>
        {columns.map((c) => (
          <div
            key={c.key}
            role="columnheader"
            className={c.className}
            aria-sort={sort.key === c.key ? (sort.descending ? "descending" : "ascending") : "none"}
          >
            {/* Tab from the list reaches the headers, so sorting works from the keyboard. */}
            <button type="button" onClick={() => props.onSort(c.key)}>
              {t(c.label)}
              {sort.key === c.key && <span aria-hidden="true">{sort.descending ? " ▾" : " ▴"}</span>}
            </button>
          </div>
        ))}
      </div>
        {props.empty !== null ? (
          <p className="empty">{props.empty}</p>
        ) : (
          <div style={{ height: entries.length * ROW, position: "relative" }}>
            {entries.slice(first, last).map((entry, i) => {
              const index = first + i;
              const selected = selection.ids.has(entry.id);
              return (
                <div
                  key={entry.id}
                  id={`row-${entry.id}`}
                  role="row"
                  aria-rowindex={index + 2}
                  aria-selected={selected}
                  className={[
                    "row",
                    selected ? "selected" : "",
                    selection.focus === entry.id ? "focused" : "",
                    entry.mtime === null ? "damaged" : "",
                  ].join(" ")}
                  style={{ top: index * ROW }}
                  onMouseDown={(e) => {
                    if (e.button === 0) props.onClick(entry.id, modeOf(e));
                  }}
                  onDoubleClick={() => props.onActivate(entry)}
                >
                  <div role="gridcell" className="c-name">
                    {kindIcon[entry.kind]}
                    <span className="name">{entry.name}</span>
                  </div>
                  <div role="gridcell" className="c-size">
                    {entry.kind === "dir" ? "" : formatSize(entry.size, lang)}
                  </div>
                  <div role="gridcell" className="c-time">
                    {formatTime(entry.mtime, lang)}
                  </div>
                  <div role="gridcell" className="c-mode">
                    {formatMode(entry.kind, entry.mode)}
                  </div>
                  <div role="gridcell" className="c-owner">
                    {entry.uid}:{entry.gid}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

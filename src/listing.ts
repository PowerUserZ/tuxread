// Sorting and selection for the file list, kept apart from React so they can be tested.
import type { EntryView } from "./api";
import type { Lang } from "./i18n";

export type SortKey = "name" | "size" | "mtime" | "mode" | "owner";
export type Sort = { key: SortKey; descending: boolean };

/** Folders first, then by `sort.key`, ties broken by name. Names compare like Explorer:
 *  case-insensitively, with numbers in numeric order ("file2" before "file10"). */
export function sortEntries(entries: EntryView[], sort: Sort, lang: Lang): EntryView[] {
  const collator = new Intl.Collator(lang, { numeric: true, sensitivity: "base" });
  const byName = (a: EntryView, b: EntryView) => collator.compare(a.name, b.name);
  const value = (e: EntryView): number => {
    switch (sort.key) {
      case "size":
        return e.size;
      case "mtime":
        return e.mtime ?? Number.NEGATIVE_INFINITY;
      case "mode":
        return e.mode;
      case "owner":
        return e.uid * 2 ** 32 + e.gid;
      case "name":
        return 0;
    }
  };
  const direction = sort.descending ? -1 : 1;
  return [...entries].sort((a, b) => {
    const folders = Number(b.kind === "dir") - Number(a.kind === "dir");
    if (folders !== 0) return folders;
    const primary = sort.key === "name" ? byName(a, b) : value(a) - value(b);
    return primary !== 0 ? direction * primary : byName(a, b);
  });
}

/** Selected entry ids, the anchor of a Shift range and the focused row (as ids). */
export type Selection = { ids: ReadonlySet<number>; anchor: number | null; focus: number | null };

export const emptySelection: Selection = { ids: new Set(), anchor: null, focus: null };

export type ClickMode = "replace" | "toggle" | "range";

/** A click on `id` in the list's current `order`: plain, Ctrl (toggle) or Shift (range). */
export function click(sel: Selection, order: number[], id: number, mode: ClickMode): Selection {
  if (mode === "toggle") {
    const ids = new Set(sel.ids);
    if (ids.has(id)) ids.delete(id);
    else ids.add(id);
    return { ids, anchor: id, focus: id };
  }
  if (mode === "range" && sel.anchor !== null) {
    return { ids: new Set(range(order, sel.anchor, id)), anchor: sel.anchor, focus: id };
  }
  return { ids: new Set([id]), anchor: id, focus: id };
}

/** Arrow keys and friends: move the focus by `delta` rows (clamped); Shift extends. */
export function move(sel: Selection, order: number[], delta: number, extend: boolean): Selection {
  if (order.length === 0) return sel;
  const at = sel.focus === null ? -1 : order.indexOf(sel.focus);
  const start = at === -1 ? (delta > 0 ? -1 : order.length) : at;
  const index = Math.min(order.length - 1, Math.max(0, start + delta));
  const id = order[index] as number;
  return click(sel, order, id, extend ? "range" : "replace");
}

export function selectAll(order: number[]): Selection {
  return { ids: new Set(order), anchor: order[0] ?? null, focus: order[0] ?? null };
}

function range(order: number[], from: number, to: number): number[] {
  const a = order.indexOf(from);
  const b = order.indexOf(to);
  if (a === -1 || b === -1) return [to];
  return order.slice(Math.min(a, b), Math.max(a, b) + 1);
}

import { describe, expect, it } from "vitest";
import type { EntryView } from "./api";
import { formatMode, formatOctal, formatSize, formatTime } from "./format";
import { pickLang, translate } from "./i18n";
import { click, emptySelection, move, selectAll, sortEntries } from "./listing";

const entry = (id: number, name: string, kind: EntryView["kind"], size = 0, mtime: number | null = 0): EntryView => ({
  id,
  name,
  kind,
  size,
  mtime,
  mode: 0o644,
  uid: 1000,
  gid: 1000,
});

describe("format", () => {
  it("sizes read like Explorer, in both languages", () => {
    expect(formatSize(0, "en")).toBe("0 bytes");
    expect(formatSize(6, "tr")).toBe("6 bayt");
    expect(formatSize(1536, "en")).toBe("1.5 KB");
    expect(formatSize(1536, "tr")).toBe("1,5 KB");
    expect(formatSize(500 * 1024 * 1024, "en")).toBe("500 MB");
    expect(formatSize(931.5 * 1024 ** 3, "en")).toBe("932 GB");
  });

  it("times before 1970 and unknown times are shown", () => {
    expect(formatTime(null, "en")).toBe("—");
    expect(formatTime(-31_536_000, "en")).toMatch(/1969/);
  });

  it("modes read like ls -l, with special bits", () => {
    expect(formatMode("file", 0o644)).toBe("-rw-r--r--");
    expect(formatMode("dir", 0o755)).toBe("drwxr-xr-x");
    expect(formatMode("file", 0o4755)).toBe("-rwsr-xr-x");
    expect(formatMode("dir", 0o1777)).toBe("drwxrwxrwt");
    expect(formatMode("file", 0o2644)).toBe("-rw-r-Sr--");
    expect(formatMode("symlink", 0o777)).toBe("lrwxrwxrwx");
    expect(formatOctal(0o755)).toBe("0755");
  });
});

describe("i18n", () => {
  it("follows the OS language unless a language was chosen", () => {
    expect(pickLang(null, "tr-TR")).toBe("tr");
    expect(pickLang(null, "en-GB")).toBe("en");
    expect(pickLang(null, "de-DE")).toBe("en");
    expect(pickLang("en", "tr-TR")).toBe("en");
    expect(pickLang("xx", "tr-TR")).toBe("tr");
  });

  it("fills in parameters and leaves unknown ones visible", () => {
    expect(translate("en", "itemsMany", { n: 3 })).toBe("3 items");
    expect(translate("tr", "itemsMany", { n: 3 })).toBe("3 öğe");
    expect(translate("en", "itemsMany")).toBe("{n} items");
  });
});

describe("sorting", () => {
  const entries = [
    entry(1, "file10.txt", "file", 30, 3),
    entry(2, "File2.txt", "file", 10, 1),
    entry(3, "zeta", "dir", 0, 2),
    entry(4, "alpha", "dir", 0, null),
  ];
  const names = (e: EntryView[]) => e.map((x) => x.name);

  it("puts folders first and orders names naturally, ignoring case", () => {
    expect(names(sortEntries(entries, { key: "name", descending: false }, "en"))).toEqual([
      "alpha",
      "zeta",
      "File2.txt",
      "file10.txt",
    ]);
    expect(names(sortEntries(entries, { key: "name", descending: true }, "en"))).toEqual([
      "zeta",
      "alpha",
      "file10.txt",
      "File2.txt",
    ]);
  });

  it("sorts by size and time, unknown times first", () => {
    expect(names(sortEntries(entries, { key: "size", descending: true }, "en"))).toEqual([
      "alpha",
      "zeta",
      "file10.txt",
      "File2.txt",
    ]);
    expect(names(sortEntries(entries, { key: "mtime", descending: false }, "en"))).toEqual([
      "alpha",
      "zeta",
      "File2.txt",
      "file10.txt",
    ]);
  });
});

describe("selection", () => {
  const order = [10, 20, 30, 40];

  it("click replaces, Ctrl+click toggles, Shift+click selects a range", () => {
    let sel = click(emptySelection, order, 20, "replace");
    expect([...sel.ids]).toEqual([20]);
    sel = click(sel, order, 40, "range");
    expect([...sel.ids]).toEqual([20, 30, 40]);
    sel = click(sel, order, 30, "toggle");
    expect([...sel.ids].sort()).toEqual([20, 40]);
  });

  it("arrow keys move and extend within bounds", () => {
    let sel = move(emptySelection, order, 1, false);
    expect(sel.focus).toBe(10);
    sel = move(sel, order, 2, true);
    expect([...sel.ids]).toEqual([10, 20, 30]);
    sel = move(sel, order, 99, false);
    expect([...sel.ids]).toEqual([40]);
    sel = move(sel, order, -99, false);
    expect(sel.focus).toBe(10);
    expect(move(emptySelection, [], 1, false)).toBe(emptySelection);
  });

  it("Ctrl+A selects everything", () => {
    expect(selectAll(order).ids.size).toBe(4);
  });
});

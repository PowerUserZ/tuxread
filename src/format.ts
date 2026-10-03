// How sizes, times and Linux permissions read in the window.
import type { EntryKind } from "./api";
import type { Lang } from "./i18n";

const locales: Record<Lang, string> = { en: "en-US", tr: "tr-TR" };
const byteUnit: Record<Lang, string> = { en: "bytes", tr: "bayt" };

/** Bytes as Windows Explorer shows them: binary units named KB, MB, GB. */
export function formatSize(bytes: number, lang: Lang): string {
  const units = [byteUnit[lang], "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  const number = new Intl.NumberFormat(locales[lang], {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(value);
  return `${number} ${units[unit]}`;
}

/** Seconds since 1970 as a local date and time; "—" when unknown. */
export function formatTime(secs: number | null, lang: Lang): string {
  if (secs === null) return "—";
  return new Intl.DateTimeFormat(locales[lang], { dateStyle: "medium", timeStyle: "short" }).format(
    new Date(secs * 1000),
  );
}

/** `ls -l` style: type, then rwx for owner, group and others with setuid, setgid, sticky. */
export function formatMode(kind: EntryKind, mode: number): string {
  const type = { file: "-", dir: "d", symlink: "l", other: "?" }[kind];
  const bit = (mask: number, c: string) => (mode & mask ? c : "-");
  const exec = (mask: number, special: number, on: string, off: string) =>
    mode & special ? (mode & mask ? on : off) : bit(mask, "x");
  return (
    type +
    bit(0o400, "r") +
    bit(0o200, "w") +
    exec(0o100, 0o4000, "s", "S") +
    bit(0o040, "r") +
    bit(0o020, "w") +
    exec(0o010, 0o2000, "s", "S") +
    bit(0o004, "r") +
    bit(0o002, "w") +
    exec(0o001, 0o1000, "t", "T")
  );
}

export function formatOctal(mode: number): string {
  return mode.toString(8).padStart(4, "0");
}

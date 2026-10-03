// The backend's commands and types (src-tauri/src). Entries are referred to by number only:
// names here are display text, never paths to send back.
import { Channel, invoke } from "@tauri-apps/api/core";

const errorCodes = [
  "declined",
  "helperGone",
  "notFound",
  "notADirectory",
  "unsupported",
  "corrupt",
  "io",
  "other",
] as const;

export type ErrorCode = (typeof errorCodes)[number];

export type CommandError = { code: ErrorCode; message: string };

export type FsView = { fsType: string; label: string; uuid: string; size: number; used: number };

/** A partition Windows skips, in PowerShell's terms: Get-Disk's Guid or Signature, and
 *  Get-Partition's Offset and GptType or MbrType now and after the fix. */
export type FixView = { table: "gpt" | "mbr"; disk: string; offset: number; from: string; to: string };

export type NodeView = {
  label: string;
  size: number;
  kind: "partition" | "volume" | "detected";
  volume: number | null;
  status: "windows" | "windowsSkips" | "later" | "unsupported" | "unrecognized" | "error" | null;
  detail: string | null;
  fix: FixView | null;
  fs: FsView | null;
  children: NodeView[];
};

export type SourceView = {
  kind: "disk" | "image";
  key: number;
  name: string;
  detail: string;
  size: number;
  open: boolean;
  nodes: NodeView[];
};

export type SourcesView = { disks: SourceView[]; images: SourceView[] };

export type EntryKind = "file" | "dir" | "symlink" | "other";

export type EntryView = {
  id: number;
  name: string;
  kind: EntryKind;
  size: number;
  mtime: number | null;
  mode: number;
  uid: number;
  gid: number;
};

export type Crumb = { id: number; name: string };

type ListEvent =
  | { event: "start"; data: { crumbs: Crumb[]; total: number } }
  | { event: "page"; data: { entries: EntryView[] } };

export type Properties = {
  name: string;
  path: string;
  kind: EntryKind;
  size: number;
  mtime: number | null;
  mode: number;
  uid: number;
  gid: number;
  inode: number;
  link: string | null;
};

export type Conflict = "keepBoth" | "skip" | "overwrite";

export type JobEvent =
  | { event: "progress"; data: { files: number; bytes: number; current: string } }
  | {
      event: "finished";
      data: {
        copied: number;
        renamed: number;
        skipped: number;
        failed: number;
        cancelled: boolean;
        error: CommandError | null;
      };
    };

export type ReportItem = {
  source: string;
  dest: string | null;
  outcome: "copied" | "renamed" | "skipped" | "failed";
  detail: string | null;
};

/** Errors from `invoke` are `CommandError`s. Anything else (a plugin's text, a DOMException
 *  with a numeric `code`) becomes `other`, so the window only ever translates codes it knows. */
export function asCommandError(e: unknown): CommandError {
  if (typeof e === "object" && e !== null && "message" in e) {
    const known = "code" in e && errorCodes.includes(e.code as ErrorCode);
    return { code: known ? (e.code as ErrorCode) : "other", message: String(e.message) };
  }
  return { code: "other", message: String(e) };
}

export const listSources = () => invoke<SourcesView>("list_sources");
export const openImage = (path: string) => invoke<SourceView>("open_image", { path });
export const openDisk = (number: number) => invoke<SourceView>("open_disk", { number });
export const stat = (volume: number, entry: number) =>
  invoke<Properties>("stat", { volume, entry });
export const cancelJob = (job: number) => invoke<void>("cancel_job", { job });
export const jobReport = (job: number) => invoke<ReportItem[]>("job_report", { job });
export const saveReport = (job: number, path: string) =>
  invoke<void>("save_report", { job, path });
export const diagnostics = () => invoke<string>("diagnostics");

/** Lists folder `dir`; `onStart` comes first, then `onPage` for each page of entries. */
export function listDir(
  volume: number,
  dir: number,
  onStart: (crumbs: Crumb[], total: number) => void,
  onPage: (entries: EntryView[]) => void,
): Promise<number> {
  const onEvent = new Channel<ListEvent>();
  onEvent.onmessage = (m) => {
    if (m.event === "start") onStart(m.data.crumbs, m.data.total);
    else onPage(m.data.entries);
  };
  return invoke<number>("list_dir", { volume, dir, onEvent });
}

/** Starts a copy job and returns its number; `onEvent` ends with a `finished` event. */
export function copy(
  volume: number,
  entries: number[],
  dest: string,
  conflict: Conflict,
  onEvent: (e: JobEvent) => void,
): Promise<number> {
  const channel = new Channel<JobEvent>();
  channel.onmessage = onEvent;
  return invoke<number>("copy", { volume, entries, dest, conflict, onEvent: channel });
}

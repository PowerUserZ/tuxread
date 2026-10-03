// Error text, properties, about, copy options and the copy report (spec §5.9).
import { open, save } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import {
  type CommandError,
  type Conflict,
  type ErrorCode,
  type Properties,
  type ReportItem,
  asCommandError,
  jobReport,
  saveReport,
} from "./api";
import { Dialog } from "./Dialog";
import { formatMode, formatOctal, formatSize, formatTime } from "./format";
import { type Key, type Lang, useI18n } from "./i18n";
import { folderPath } from "./paths";
import type { FixChoice } from "./Sidebar";
import { fixScript } from "./windowsFix";

const errorKey: Record<ErrorCode, Key> = {
  declined: "errDeclined",
  helperGone: "errHelperGone",
  notFound: "errNotFound",
  notADirectory: "errNotADirectory",
  unsupported: "errUnsupported",
  corrupt: "errCorrupt",
  io: "errIo",
  other: "errOther",
};

/** A translated summary of `error`; its English detail goes underneath. */
export function ErrorText({ error }: { error: CommandError }) {
  const { t } = useI18n();
  return (
    <>
      <strong>{t(errorKey[error.code])}</strong>
      <span className="detail">{t("details", { message: error.message })}</span>
    </>
  );
}

export const kindKey = { file: "kindFile", dir: "kindDir", symlink: "kindSymlink", other: "kindOther" } as const;

export function PropertiesDialog({ props, onClose }: { props: Properties; onClose: () => void }) {
  const { t, lang } = useI18n();
  const rows: [Key, string][] = [
    ["propName", props.name],
    ["propPath", props.path],
    ["propKind", t(kindKey[props.kind])],
    ["propSize", props.kind === "dir" ? "—" : `${formatSize(props.size, lang)} (${props.size.toLocaleString(lang)})`],
    ["propModified", formatTime(props.mtime, lang)],
    ["propPermissions", `${formatMode(props.kind, props.mode)} (${formatOctal(props.mode)})`],
    ["propOwner", t("ownerIds", { uid: props.uid, gid: props.gid })],
    ["propInode", String(props.inode)],
  ];
  if (props.link !== null) rows.push(["propLink", props.link]);
  return (
    <Dialog
      title={t("properties")}
      onClose={onClose}
      actions={
        <button type="button" className="primary" onClick={onClose}>
          {t("close")}
        </button>
      }
    >
      <dl className="props">
        {rows.map(([key, value]) => (
          <div key={key}>
            <dt>{t(key)}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
    </Dialog>
  );
}

export function AboutDialog({
  stored,
  onLanguage,
  onClose,
}: {
  stored: Lang | null;
  onLanguage: (lang: Lang | null) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  return (
    <Dialog
      title={t("aboutTitle")}
      onClose={onClose}
      actions={
        <button type="button" className="primary" onClick={onClose}>
          {t("close")}
        </button>
      }
    >
      <p className="version">{t("version", { version: __APP_VERSION__ })}</p>
      <p>{t("aboutText")}</p>
      <p>{t("credit")}</p>
      <p>{t("license")}</p>
      <p>{t("builtWith")}</p>
      <label className="field">
        <span className="label">{t("language")}</span>
        <select
          value={stored ?? ""}
          onChange={(e) => onLanguage(e.target.value === "" ? null : (e.target.value as Lang))}
        >
          <option value="">{t("langSystem")}</option>
          <option value="en">English</option>
          <option value="tr">Türkçe</option>
        </select>
      </label>
    </Dialog>
  );
}

export function FixDialog({ choice, onClose }: { choice: FixChoice; onClose: () => void }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState<"copied" | "failed" | null>(null);
  const script = fixScript(choice.fix, t("fixNotFound"));
  return (
    <Dialog
      title={t("fixTitle")}
      wide
      onClose={onClose}
      actions={
        <>
          <button
            type="button"
            onClick={() => navigator.clipboard.writeText(script).then(() => setCopied("copied"), () => setCopied("failed"))}
          >
            {copied === "copied" ? t("fixCopied") : copied === "failed" ? t("fixCopyFailed") : t("fixCopy")}
          </button>
          <button type="button" className="primary" onClick={onClose}>
            {t("close")}
          </button>
        </>
      }
    >
      <p>{t("fixWhy", { fs: choice.fs, type: choice.partType })}</p>
      <ol className="steps">
        <li>{t("fixStep1")}</li>
        <li>{t("fixStep2")}</li>
        <li>{t("fixStep3")}</li>
      </ol>
      <pre className="script">{script}</pre>
      <p>{t("fixSafe")}</p>
    </Dialog>
  );
}

export function CopyDialog({
  count,
  onCopy,
  onClose,
}: {
  count: number;
  onCopy: (dest: string, conflict: Conflict) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [dest, setDest] = useState("");
  const [conflict, setConflict] = useState<Conflict>("keepBoth");
  const choose = async () => {
    const folder = await open({ directory: true, multiple: false });
    if (typeof folder === "string") setDest(folder);
  };
  const options: [Conflict, Key][] = [
    ["keepBoth", "keepBoth"],
    ["skip", "skip"],
    ["overwrite", "overwrite"],
  ];
  return (
    <Dialog
      title={count === 1 ? t("copyTitleOne") : t("copyTitleMany", { n: count })}
      onClose={onClose}
      onSubmit={() => folderPath(dest) !== "" && onCopy(folderPath(dest), conflict)}
      actions={
        <>
          <button type="button" onClick={onClose}>
            {t("cancel")}
          </button>
          <button type="submit" className="primary" disabled={folderPath(dest) === ""}>
            {t("copy")}
          </button>
        </>
      }
    >
      <label className="field">
        <span className="label">{t("destination")}</span>
        <span className="dest">
          <input
            name="dest"
            value={dest}
            placeholder={t("noFolder")}
            spellCheck={false}
            onChange={(e) => setDest(e.target.value)}
          />
          <button type="button" onClick={() => void choose()}>
            {t("chooseFolder")}
          </button>
        </span>
      </label>
      <fieldset className="field">
        <legend className="label">{t("ifExists")}</legend>
        {options.map(([value, label]) => (
          <label key={value} className="radio">
            <input
              type="radio"
              name="conflict"
              value={value}
              checked={conflict === value}
              onChange={() => setConflict(value)}
            />
            {t(label)}
          </label>
        ))}
      </fieldset>
    </Dialog>
  );
}

const outcomeKey: Record<ReportItem["outcome"], Key> = {
  copied: "outcomeCopied",
  renamed: "outcomeRenamed",
  skipped: "outcomeSkipped",
  failed: "outcomeFailed",
};

/** Failures first, then the other items that need a look; plain copies last. */
const rank: Record<ReportItem["outcome"], number> = { failed: 0, skipped: 1, renamed: 2, copied: 3 };

/** At most this many report lines are shown; "Save as text" writes them all. */
const SHOWN = 500;

export function ReportDialog({ job, onClose }: { job: number; onClose: () => void }) {
  const { t } = useI18n();
  const [items, setItems] = useState<ReportItem[] | null>(null);
  const [note, setNote] = useState<string | CommandError | null>(null);
  useEffect(() => {
    jobReport(job).then(setItems, (e) => setNote(asCommandError(e)));
  }, [job]);
  const count = (o: ReportItem["outcome"]) => items?.filter((i) => i.outcome === o).length ?? 0;
  const ordered = items ? [...items].sort((a, b) => rank[a.outcome] - rank[b.outcome]) : [];
  const saveText = async () => {
    const path = await save({ defaultPath: "TuxRead copy report.txt", filters: [{ name: "Text", extensions: ["txt"] }] });
    if (!path) return;
    saveReport(job, path).then(
      () => setNote(t("reportSaved")),
      (e) => setNote(asCommandError(e)),
    );
  };
  return (
    <Dialog
      title={t("report")}
      wide
      onClose={onClose}
      actions={
        <>
          <button type="button" onClick={() => void saveText()} disabled={!items}>
            {t("saveReport")}
          </button>
          <button type="button" className="primary" onClick={onClose}>
            {t("close")}
          </button>
        </>
      }
    >
      {items && (
        <p>
          {t("finishedSummary", {
            copied: count("copied"),
            renamed: count("renamed"),
            skipped: count("skipped"),
            failed: count("failed"),
          })}
        </p>
      )}
      {note !== null && (
        <p role="status" className={typeof note === "string" ? "note" : "note error"}>
          {typeof note === "string" ? note : <ErrorText error={note} />}
        </p>
      )}
      <ul className="report">
        {ordered.slice(0, SHOWN).map((item, i) => (
          <li key={i} className={item.outcome}>
            <span className="outcome">{t(outcomeKey[item.outcome])}</span>
            <span className="source">{item.source}</span>
            {item.detail && (
              <span className="why">
                {item.outcome === "renamed" ? t("renamedTo", { name: item.detail }) : item.detail}
              </span>
            )}
          </li>
        ))}
      </ul>
      {ordered.length > SHOWN && <p className="note">… {ordered.length - SHOWN}</p>}
    </Dialog>
  );
}

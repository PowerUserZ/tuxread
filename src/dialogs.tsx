// Error text, properties and about (spec §5.9).
import type { CommandError, ErrorCode, Properties } from "./api";
import { Dialog } from "./Dialog";
import { formatMode, formatOctal, formatSize, formatTime } from "./format";
import { type Key, type Lang, useI18n } from "./i18n";

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

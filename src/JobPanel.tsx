// Copies in progress and just finished, with Cancel and the report (spec §5.9).
import type { JobEvent } from "./api";
import { ErrorText } from "./dialogs";
import { formatSize } from "./format";
import { useI18n } from "./i18n";

export type Job = {
  /** Known before the backend's number arrives. */
  key: number;
  /** The backend's job number, once `copy` has returned. */
  id: number | null;
  dest: string;
  progress: { files: number; bytes: number; current: string };
  finished: Extract<JobEvent, { event: "finished" }>["data"] | null;
};

type Props = {
  jobs: Job[];
  onCancel: (id: number) => void;
  onReport: (id: number) => void;
  onDismiss: (key: number) => void;
};

export function JobPanel({ jobs, onCancel, onReport, onDismiss }: Props) {
  const { t, lang } = useI18n();
  if (jobs.length === 0) return null;
  return (
    <section className="jobs" aria-label={t("copyTo")}>
      {jobs.map((job) => {
        const done = job.finished;
        return (
          <div key={job.key} className="job" role="status">
            <div className="job-text">
              <span className="job-title">{t(done === null ? "copying" : "copyDone", { dest: job.dest })}</span>
              {done === null ? (
                <span className="job-line">
                  {t("copied", { files: job.progress.files.toLocaleString(lang), bytes: formatSize(job.progress.bytes, lang) })}
                  <span className="current">{job.progress.current}</span>
                </span>
              ) : done.error ? (
                <span className="job-line error">
                  {t("copyFailed")} <ErrorText error={done.error} />
                </span>
              ) : (
                <span className={done.failed > 0 ? "job-line warn" : "job-line"}>
                  {done.cancelled ? `${t("cancelled")}. ` : ""}
                  {t("finishedSummary", {
                    copied: done.copied.toLocaleString(lang),
                    renamed: done.renamed.toLocaleString(lang),
                    skipped: done.skipped.toLocaleString(lang),
                    failed: done.failed.toLocaleString(lang),
                  })}
                </span>
              )}
            </div>
            {done === null ? (
              <>
                <div className="bar" aria-hidden="true">
                  <span />
                </div>
                <button type="button" disabled={job.id === null} onClick={() => job.id !== null && onCancel(job.id)}>
                  {t("cancel")}
                </button>
              </>
            ) : (
              <>
                {!done.error && job.id !== null && (
                  <button type="button" onClick={() => job.id !== null && onReport(job.id)}>
                    {t("showReport")}
                  </button>
                )}
                <button type="button" className="ghost" onClick={() => onDismiss(job.key)}>
                  {t("dismiss")}
                </button>
              </>
            )}
          </div>
        );
      })}
    </section>
  );
}

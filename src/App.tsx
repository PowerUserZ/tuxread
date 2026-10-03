// The window: sources on the left; path, toolbar, entries, copy jobs and status on the right.
import { open } from "@tauri-apps/plugin-dialog";
import { type KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  type CommandError,
  type Conflict,
  type Crumb,
  type EntryView,
  type Properties,
  type SourcesView,
  asCommandError,
  cancelJob,
  copy,
  diagnostics,
  listDir,
  listSources,
  openDisk,
  openImage,
  stat,
} from "./api";
import { AboutDialog, CopyDialog, ErrorText, PropertiesDialog, ReportDialog } from "./dialogs";
import { FileList } from "./FileList";
import { formatSize } from "./format";
import { I18n, type Lang, type T, pickLang, translate } from "./i18n";
import { BackIcon, RefreshIcon, UpIcon } from "./icons";
import { type Job, JobPanel } from "./JobPanel";
import { type Selection, type Sort, type SortKey, click, emptySelection, move, selectAll, sortEntries } from "./listing";
import { Sidebar, type VolumeChoice } from "./Sidebar";

type Place = { volume: number; dir: number };
type Open =
  | null
  | { kind: "copy" }
  | { kind: "report"; job: number }
  | { kind: "props"; props: Properties }
  | { kind: "about" };

const LANG_KEY = "tuxread.language";

function readStoredLang(): Lang | null {
  try {
    const value = localStorage.getItem(LANG_KEY);
    return value === "en" || value === "tr" ? value : null;
  } catch {
    return null;
  }
}

function storeLang(lang: Lang | null) {
  try {
    if (lang === null) localStorage.removeItem(LANG_KEY);
    else localStorage.setItem(LANG_KEY, lang);
  } catch {
    // Storage may be unavailable; the choice then lasts until the app closes.
  }
}

export function App() {
  const [stored, setStored] = useState<Lang | null>(readStoredLang);
  const lang = pickLang(stored, navigator.language);
  const t: T = useCallback((key, params) => translate(lang, key, params), [lang]);
  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  const [sources, setSources] = useState<SourcesView>({ disks: [], images: [] });
  const [opening, setOpening] = useState<number | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [volume, setVolume] = useState<VolumeChoice | null>(null);
  const [place, setPlace] = useState<Place | null>(null);
  const [history, setHistory] = useState<Place[]>([]);
  const [crumbs, setCrumbs] = useState<Crumb[]>([]);
  const [entries, setEntries] = useState<EntryView[]>([]);
  const [loading, setLoading] = useState(false);
  const [sort, setSort] = useState<Sort>({ key: "name", descending: false });
  const [selection, setSelection] = useState<Selection>(emptySelection);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [dialog, setDialog] = useState<Open>(null);
  const request = useRef(0);
  const nextJob = useRef(0);

  const fail = useCallback((e: unknown) => setError(asCommandError(e)), []);
  const refreshSources = useCallback(() => listSources().then(setSources, fail), [fail]);
  useEffect(() => {
    void refreshSources();
  }, [refreshSources]);

  const sorted = useMemo(() => sortEntries(entries, sort, lang), [entries, sort, lang]);
  const order = useMemo(() => sorted.map((e) => e.id), [sorted]);

  /** Lists `p`; pages of a listing the user has since left are ignored. */
  const load = useCallback(
    (p: Place) => {
      const req = ++request.current;
      setLoading(true);
      setEntries([]);
      setSelection(emptySelection);
      setError(null);
      const got: EntryView[] = [];
      listDir(
        p.volume,
        p.dir,
        (c) => req === request.current && setCrumbs(c),
        (page) => {
          if (req !== request.current) return;
          got.push(...page);
          setEntries([...got]);
        },
      ).then(
        () => req === request.current && setLoading(false),
        (e) => {
          if (req !== request.current) return;
          setLoading(false);
          fail(e);
        },
      );
    },
    [fail],
  );

  const go = (p: Place) => {
    if (place) setHistory((h) => [...h, place]);
    setPlace(p);
    load(p);
  };

  const chooseVolume = (choice: VolumeChoice) => {
    setVolume(choice);
    setHistory([]);
    setCrumbs([]);
    const p = { volume: choice.volume, dir: 0 };
    setPlace(p);
    load(p);
  };

  const back = () => {
    const prev = history.at(-1);
    if (!prev) return;
    setHistory((h) => h.slice(0, -1));
    setPlace(prev);
    load(prev);
  };

  const parent = crumbs.length > 1 ? crumbs[crumbs.length - 2] : undefined;
  const up = () => place && parent && go({ volume: place.volume, dir: parent.id });
  const refresh = () => place && load(place);

  const target = () => {
    const id = selection.focus ?? [...selection.ids][0];
    return sorted.find((e) => e.id === id);
  };

  const showProperties = (entry = target()) => {
    if (!place || !entry) return;
    stat(place.volume, entry.id).then((props) => setDialog({ kind: "props", props }), fail);
  };

  const activate = (entry: EntryView | undefined) => {
    if (!entry || !place) return;
    if (entry.kind === "dir") go({ volume: place.volume, dir: entry.id });
    else showProperties(entry);
  };

  const copyTo = () => {
    if (place && selection.ids.size > 0) setDialog({ kind: "copy" });
  };

  const startCopy = (dest: string, conflict: Conflict) => {
    if (!place) return;
    setDialog(null);
    // Events can arrive before `copy` returns the job's number: they are matched by `key`.
    const key = ++nextJob.current;
    const update = (change: (job: Job) => Job) =>
      setJobs((all) => all.map((j) => (j.key === key ? change(j) : j)));
    setJobs((all) => [
      ...all,
      { key, id: null, dest, progress: { files: 0, bytes: 0, current: "" }, finished: null },
    ]);
    copy(place.volume, [...selection.ids], dest, conflict, (e) => {
      if (e.event === "progress") update((j) => ({ ...j, progress: e.data }));
      else update((j) => ({ ...j, finished: e.data }));
    }).then(
      (id) => update((j) => ({ ...j, id })),
      (e) => {
        const err = asCommandError(e);
        update((j) => ({
          ...j,
          finished: { copied: 0, renamed: 0, skipped: 0, failed: 0, cancelled: false, error: err },
        }));
      },
    );
  };

  const onOpenDisk = (number: number) => {
    setOpening(number);
    setError(null);
    openDisk(number)
      .then(() => refreshSources(), fail)
      .finally(() => setOpening(null));
  };

  const onOpenImage = async () => {
    const path = await open({ multiple: false, directory: false });
    if (typeof path !== "string") return;
    setError(null);
    openImage(path).then(() => refreshSources(), fail);
  };

  const onDiagnostics = () => {
    diagnostics()
      .then((text) => navigator.clipboard.writeText(text))
      .then(() => setNotice(t("diagnosticsCopied")), fail);
  };

  const onSort = (key: SortKey) =>
    setSort((s) => ({ key, descending: s.key === key ? !s.descending : false }));

  const onListKey = (e: KeyboardEvent<HTMLDivElement>, pageRows: number) => {
    const step = (delta: number) => setSelection((s) => move(s, order, delta, e.shiftKey));
    if (e.key === "ArrowDown") step(1);
    else if (e.key === "ArrowUp") step(-1);
    else if (e.key === "PageDown") step(pageRows);
    else if (e.key === "PageUp") step(-pageRows);
    else if (e.key === "Home") step(-order.length);
    else if (e.key === "End") step(order.length);
    else if (e.key === "Enter" && e.altKey) showProperties();
    else if (e.key === "Enter") activate(target());
    else if (e.key === "Backspace") up();
    else if (e.ctrlKey && e.key.toLowerCase() === "a") setSelection(selectAll(order));
    else if (e.ctrlKey && e.key.toLowerCase() === "c") copyTo();
    else return;
    e.preventDefault();
  };

  // Alt+Left goes back and F5 refreshes from anywhere in the window.
  const keys = useRef({ back, refresh });
  keys.current = { back, refresh };
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.altKey && e.key === "ArrowLeft") keys.current.back();
      else if (e.key === "F5") keys.current.refresh();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const fs = volume?.node.fs;
  const emptyText = !place
    ? t("chooseVolume")
    : sorted.length > 0
      ? null
      : loading
        ? t("loading")
        : error
          ? ""
          : t("emptyFolder");

  return (
    <I18n.Provider value={{ lang, t }}>
      <div className="app">
        <Sidebar
          sources={sources}
          current={place?.volume ?? null}
          opening={opening}
          onOpenDisk={onOpenDisk}
          onOpenImage={() => void onOpenImage()}
          onChooseVolume={chooseVolume}
          onAbout={() => setDialog({ kind: "about" })}
          onDiagnostics={onDiagnostics}
        />
        <main className="main">
          <header className="toolbar">
            <div className="nav">
              <button type="button" className="icon-button" aria-label={t("back")} title={t("back")} disabled={history.length === 0} onClick={back}>
                <BackIcon />
              </button>
              <button type="button" className="icon-button" aria-label={t("up")} title={t("up")} disabled={!parent} onClick={up}>
                <UpIcon />
              </button>
              <button type="button" className="icon-button" aria-label={t("refresh")} title={t("refresh")} disabled={!place} onClick={refresh}>
                <RefreshIcon />
              </button>
            </div>
            <nav className="pathbar" aria-label={t("propPath")}>
              <ol>
                {volume &&
                  crumbs.map((c, i) => (
                    <li key={c.id}>
                      <button
                        type="button"
                        aria-current={i === crumbs.length - 1 ? "page" : undefined}
                        onClick={() => place && i < crumbs.length - 1 && go({ volume: place.volume, dir: c.id })}
                      >
                        {i === 0 ? volume.node.label : c.name}
                      </button>
                    </li>
                  ))}
              </ol>
            </nav>
            <div className="actions">
              <button type="button" className="primary" disabled={selection.ids.size === 0} onClick={copyTo}>
                {t("copyTo")}
              </button>
              <button type="button" disabled={selection.ids.size === 0} onClick={() => showProperties()}>
                {t("properties")}
              </button>
            </div>
          </header>
          {error && (
            <div className="banner error" role="alert">
              <p>
                <ErrorText error={error} />
              </p>
              <button type="button" className="ghost" onClick={() => setError(null)}>
                {t("dismiss")}
              </button>
            </div>
          )}
          {notice && (
            <div className="banner" role="status">
              <p>{notice}</p>
              <button type="button" className="ghost" onClick={() => setNotice(null)}>
                {t("dismiss")}
              </button>
            </div>
          )}
          <FileList
            entries={sorted}
            selection={selection}
            sort={sort}
            label={crumbs.at(-1)?.name || volume?.node.label || t("chooseVolume")}
            empty={emptyText}
            onSort={onSort}
            onClick={(id, mode) => setSelection((s) => click(s, order, id, mode))}
            onActivate={activate}
            onKeyDown={onListKey}
          />
          <JobPanel
            jobs={jobs}
            onCancel={(id) => void cancelJob(id).catch(fail)}
            onReport={(id) => setDialog({ kind: "report", job: id })}
            onDismiss={(key) => setJobs((all) => all.filter((j) => j.key !== key))}
          />
          <footer className="statusbar">
            <span>
              {place && (sorted.length === 1 ? t("itemsOne") : t("itemsMany", { n: sorted.length.toLocaleString(lang) }))}
              {selection.ids.size > 0 && `, ${t("selected", { n: selection.ids.size.toLocaleString(lang) })}`}
            </span>
            {fs && (
              <span>
                {t("fsSummary", {
                  type: fs.label ? `${fs.fsType} “${fs.label}”` : fs.fsType,
                  used: formatSize(fs.used, lang),
                  size: formatSize(fs.size, lang),
                })}
              </span>
            )}
          </footer>
        </main>
        {dialog?.kind === "copy" && (
          <CopyDialog count={selection.ids.size} onCopy={startCopy} onClose={() => setDialog(null)} />
        )}
        {dialog?.kind === "report" && <ReportDialog job={dialog.job} onClose={() => setDialog(null)} />}
        {dialog?.kind === "props" && <PropertiesDialog props={dialog.props} onClose={() => setDialog(null)} />}
        {dialog?.kind === "about" && (
          <AboutDialog
            stored={stored}
            onLanguage={(l) => {
              storeLang(l);
              setStored(l);
            }}
            onClose={() => setDialog(null)}
          />
        )}
      </div>
    </I18n.Provider>
  );
}

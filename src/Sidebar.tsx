// Disks and images, with what each holds (spec §5.9). Each opened source draws its partitions
// as a bar, sized like on disk and colored by what TuxRead can do with them.
import type { KeyboardEvent } from "react";
import type { NodeView, SourceView, SourcesView } from "./api";
import { formatSize } from "./format";
import { type Key, useI18n } from "./i18n";
import { DiskIcon, ImageIcon, LockIcon, PlusIcon } from "./icons";

export type VolumeChoice = { volume: number; node: NodeView };

type Props = {
  sources: SourcesView;
  current: number | null;
  opening: number | null;
  onOpenDisk: (number: number) => void;
  onOpenImage: () => void;
  onChooseVolume: (choice: VolumeChoice) => void;
  onAbout: () => void;
  onDiagnostics: () => void;
};

/** What a top-level node holds, for its color on the bar. */
function tone(node: NodeView): string {
  const leaf = node.kind === "partition" ? node.children[0] : node;
  if (!leaf) return "other";
  if (leaf.kind === "volume") return "linux";
  if (leaf.status === "windows") return "windows";
  if (leaf.status === "later") return "later";
  return "other";
}

function DiskBar({ source }: { source: SourceView }) {
  return (
    <div className="diskbar" aria-hidden="true">
      {source.nodes.map((node, i) => (
        <span
          key={i}
          className={`seg ${tone(node)}`}
          style={{ flexGrow: Math.max(node.size / Math.max(source.size, 1), 0.02) }}
        />
      ))}
      <span className="seg free" style={{ flexGrow: 0.0001 }} />
    </div>
  );
}

const statusKey: Record<NonNullable<NodeView["status"]>, Key> = {
  windows: "statusWindows",
  later: "statusLater",
  unsupported: "statusUnsupported",
  unrecognized: "statusUnrecognized",
  error: "statusError",
};

/** Arrow keys, Home and End move between the tree's items; Enter and Space choose. */
function onTreeKey(e: KeyboardEvent<HTMLElement>) {
  const items = [...e.currentTarget.querySelectorAll<HTMLElement>("[role=treeitem]")];
  const at = items.indexOf(document.activeElement as HTMLElement);
  const go = (i: number) => items[Math.max(0, Math.min(items.length - 1, i))]?.focus();
  if (e.key === "ArrowDown") go(at + 1);
  else if (e.key === "ArrowUp") go(at - 1);
  else if (e.key === "Home") go(0);
  else if (e.key === "End") go(items.length - 1);
  else if (e.key === "Enter" || e.key === " ") (document.activeElement as HTMLElement)?.click();
  else return;
  e.preventDefault();
}

export function Sidebar(props: Props) {
  const { t, lang } = useI18n();
  const { sources, current, opening } = props;

  const nodeItems = (nodes: NodeView[], level: number) =>
    nodes.map((node, i) => (
      <li key={i} role="none">
        {node.kind === "volume" && node.volume !== null ? (
          <div
            role="treeitem"
            aria-level={level}
            aria-selected={current === node.volume}
            tabIndex={-1}
            className={current === node.volume ? "item volume current" : "item volume"}
            onClick={() => props.onChooseVolume({ volume: node.volume as number, node })}
          >
            <span className="item-name">{node.label}</span>
            <span className="item-size">{formatSize(node.size, lang)}</span>
          </div>
        ) : (
          <div role="treeitem" aria-level={level} tabIndex={-1} className={`item ${node.kind}`}>
            <span className="item-name">{node.label}</span>
            {node.status ? (
              <span className="item-note">{t(statusKey[node.status], { detail: node.detail ?? "" })}</span>
            ) : (
              <span className="item-size">{formatSize(node.size, lang)}</span>
            )}
          </div>
        )}
        {node.children.length > 0 && (
          <ul role="group">{nodeItems(node.children, level + 1)}</ul>
        )}
      </li>
    ));

  // The tree is one Tab stop: its first item; arrow keys move within it.
  const first = sources.disks[0] ?? sources.images[0];

  const sourceItem = (source: SourceView) => {
    const locked = source.kind === "disk" && !source.open;
    const busy = source.kind === "disk" && opening === source.key;
    return (
      <li key={`${source.kind}${source.key}`} role="none" className="source">
        <div
          role="treeitem"
          aria-level={1}
          tabIndex={source === first ? 0 : -1}
          aria-busy={busy}
          title={locked ? t("diskLocked") : source.detail}
          className={locked ? "item source-head locked" : "item source-head"}
          onClick={locked && !busy ? () => props.onOpenDisk(source.key) : undefined}
        >
          {source.kind === "disk" ? <DiskIcon /> : <ImageIcon />}
          <span className="item-name">{source.name}</span>
          {locked && <LockIcon />}
          <span className="item-size">{busy ? t("opening") : formatSize(source.size, lang)}</span>
        </div>
        {source.open && <DiskBar source={source} />}
        {source.open && source.nodes.length === 0 && <p className="item-note pad">{t("nothingFound")}</p>}
        {source.nodes.length > 0 && <ul role="group">{nodeItems(source.nodes, 2)}</ul>}
      </li>
    );
  };

  return (
    <nav className="sidebar" aria-label={t("disks")}>
      <div className="tree-scroll">
        <ul role="tree" aria-label={`${t("disks")}, ${t("images")}`} onKeyDown={onTreeKey}>
          <li role="none" className="heading">
            {t("disks")}
          </li>
          {sources.disks.map(sourceItem)}
          <li role="none" className="heading">
            {t("images")}
          </li>
          {sources.images.map(sourceItem)}
        </ul>
        <button type="button" className="ghost open-image" onClick={props.onOpenImage}>
          <PlusIcon />
          {t("openImage")}
        </button>
      </div>
      <div className="sidebar-foot">
        <button type="button" className="link" onClick={props.onAbout}>
          {t("about")}
        </button>
        <button type="button" className="link" onClick={props.onDiagnostics}>
          {t("copyDiagnostics")}
        </button>
      </div>
    </nav>
  );
}

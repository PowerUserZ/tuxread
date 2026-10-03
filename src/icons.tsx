// Small line icons, drawn on a 16-unit grid; they inherit the text color.
import type { ReactNode } from "react";

function Icon({ children }: { children: ReactNode }) {
  return (
    <svg
      className="icon"
      viewBox="0 0 16 16"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export const BackIcon = () => (
  <Icon>
    <path d="M10 3 5 8l5 5" />
  </Icon>
);

export const UpIcon = () => (
  <Icon>
    <path d="M8 13V3M3.5 7.5 8 3l4.5 4.5" />
  </Icon>
);

export const RefreshIcon = () => (
  <Icon>
    <path d="M13 8a5 5 0 1 1-1.6-3.7M13 2.5v3h-3" />
  </Icon>
);

export const LockIcon = () => (
  <Icon>
    <rect x="3.5" y="7" width="9" height="6.5" rx="1.2" />
    <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
  </Icon>
);

export const DiskIcon = () => (
  <Icon>
    <rect x="2" y="4" width="12" height="8" rx="1.5" />
    <path d="M4.5 9.5h4" />
    <circle cx="11" cy="9.5" r=".6" fill="currentColor" />
  </Icon>
);

export const ImageIcon = () => (
  <Icon>
    <path d="M4 2h5l3 3v9H4z" />
    <path d="M9 2v3h3" />
    <circle cx="8" cy="10" r="1.6" />
  </Icon>
);

export const FolderIcon = () => (
  <Icon>
    <path d="M2 4.5V12a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1H8L6.5 3.5H3a1 1 0 0 0-1 1Z" />
  </Icon>
);

export const FileIcon = () => (
  <Icon>
    <path d="M4 2h5l3 3v9H4z" />
    <path d="M9 2v3h3" />
  </Icon>
);

export const LinkIcon = () => (
  <Icon>
    <path d="M7 9a2.5 2.5 0 0 0 3.5 0l2-2a2.5 2.5 0 0 0-3.5-3.5l-.8.8" />
    <path d="M9 7a2.5 2.5 0 0 0-3.5 0l-2 2A2.5 2.5 0 0 0 7 12.5l.8-.8" />
  </Icon>
);

export const SpecialIcon = () => (
  <Icon>
    <circle cx="8" cy="8" r="5" />
    <path d="M8 5.5v3M8 10.5v.01" />
  </Icon>
);

export const PlusIcon = () => (
  <Icon>
    <path d="M8 3v10M3 8h10" />
  </Icon>
);

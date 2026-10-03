// A modal built on <dialog>: the browser traps focus, and Escape closes it.
import { type ReactNode, useEffect, useId, useRef } from "react";

export function Dialog({
  title,
  onClose,
  children,
  actions,
  wide = false,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  actions: ReactNode;
  wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
  }, []);
  return (
    <dialog
      ref={ref}
      className={wide ? "dialog wide" : "dialog"}
      aria-labelledby={titleId}
      onClose={onClose}
    >
      <h2 id={titleId}>{title}</h2>
      <div className="dialog-body">{children}</div>
      <div className="dialog-actions">{actions}</div>
    </dialog>
  );
}

// Accessible modal dialog: focus trap, Escape to close, focus restoration.
// Never uses browser-native alert/confirm (those block the Tauri event loop).

import React from "react";

export function Dialog({
  title,
  onClose,
  children,
  danger,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  danger?: boolean;
}) {
  const ref = React.useRef<HTMLDivElement>(null);
  const previouslyFocused = React.useRef<HTMLElement | null>(null);

  React.useEffect(() => {
    previouslyFocused.current = document.activeElement as HTMLElement | null;
    // Focus the dialog container so Escape works immediately.
    ref.current?.focus();
    return () => previouslyFocused.current?.focus();
  }, []);

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose();
      return;
    }
    if (e.key === "Tab" && ref.current) {
      const focusables = ref.current.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])'
      );
      if (focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        className={danger ? "dialog dialog-danger" : "dialog"}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        ref={ref}
        onKeyDown={onKeyDown}
      >
        <header className="dialog-head">
          <h2>{title}</h2>
          <button onClick={onClose} aria-label="關閉" title="關閉">
            ✕
          </button>
        </header>
        <div className="dialog-body">{children}</div>
      </div>
    </div>
  );
}

/** A two-step confirmation button for hard-to-reverse actions. */
export function ConfirmButton({
  label,
  confirmLabel,
  onConfirm,
  className,
  disabled,
}: {
  label: string;
  confirmLabel: string;
  onConfirm: () => void;
  className?: string;
  disabled?: boolean;
}) {
  const [arming, setArming] = React.useState(false);
  React.useEffect(() => {
    if (!arming) return;
    const t = setTimeout(() => setArming(false), 5000);
    return () => clearTimeout(t);
  }, [arming]);
  if (!arming) {
    return (
      <button className={className} disabled={disabled} onClick={() => setArming(true)}>
        {label}
      </button>
    );
  }
  return (
    <span className="row">
      <button
        className={className ? `${className} danger` : "danger"}
        onClick={() => {
          setArming(false);
          onConfirm();
        }}
      >
        {confirmLabel}
      </button>
      <button onClick={() => setArming(false)}>取消</button>
    </span>
  );
}

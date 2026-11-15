import { useEffect, useRef, type ReactNode } from 'react';

/** Native modal dialog: focuses the first field, closes on Esc and gives focus back to the trigger. */
export function Modal({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    const trigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (dialog && !dialog.open) {
      if (typeof dialog.showModal === 'function') dialog.showModal();
      else dialog.setAttribute('open', '');
    }
    dialog?.querySelector<HTMLElement>('input, select, textarea')?.focus();
    return () => trigger?.focus();
  }, []);
  return (
    <dialog
      ref={ref}
      className="modal card"
      role="dialog"
      aria-modal="true"
      aria-labelledby="modal-title"
      onCancel={(event) => { event.preventDefault(); onClose(); }}
      onKeyDown={(event) => { if (event.key === 'Escape') { event.preventDefault(); onClose(); } }}
    >
      <h2 id="modal-title">{title}</h2>
      {children}
    </dialog>
  );
}

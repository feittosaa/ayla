import { useState, type ReactNode } from 'react';

/** Exclusões sempre em dois passos: o botão pede confirmação, dizendo o que acontece com os dados. */
export default function Confirm({ children, question, confirm = 'Confirmar', onConfirm, disabled, className, title, ariaLabel }: {
  children: ReactNode; question: string; confirm?: string; onConfirm: () => void; disabled?: boolean; className?: string; title?: string; ariaLabel?: string;
}) {
  const [asking, setAsking] = useState(false);
  if (asking) return <span className="confirm-inline" role="group" aria-label={question} onClick={e => e.stopPropagation()}>
    <span className="confirm-question">{question}</span>
    <button type="button" className="danger-action" onClick={() => { setAsking(false); onConfirm(); }}>{confirm}</button>
    <button type="button" className="ghost-button" onClick={() => setAsking(false)}>Cancelar</button>
  </span>;
  return <button type="button" className={className} disabled={disabled} title={title} aria-label={ariaLabel} onClick={e => { e.stopPropagation(); setAsking(true); }}>{children}</button>;
}

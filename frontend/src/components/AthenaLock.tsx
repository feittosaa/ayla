import { useState, type FormEvent } from 'react';
import { LockIcon } from './Icons';
import { errorText } from '../services/Desktop';

/** O que o diálogo faz com a pasta: proteger, abrir, trocar a senha ou tirar a proteção. */
export type LockMode = 'protect' | 'unlock' | 'password' | 'unprotect';
export const MIN_PASSWORD = 8;
/** Depois da senha, a pasta fica livre por 10 minutos; passado isso, bloqueia ao sair dela. */
export const UNLOCK_GRACE_MS = 10 * 60_000;

const titles: Record<LockMode, string> = {
  protect: 'Proteger com senha', unlock: 'Pasta protegida', password: 'Trocar a senha', unprotect: 'Tirar a proteção',
};

/** Formulário de senha das pastas protegidas da Athena (num diálogo ou dentro de uma aba). */
function LockForm({ mode, folder, git, onSubmit, onCancel, autoFocus = true }: {
  mode: LockMode; folder: string; git?: boolean; autoFocus?: boolean;
  onSubmit: (password: string, next: string) => Promise<void>; onCancel?: () => void;
}) {
  const [password, setPassword] = useState('');
  const [next, setNext] = useState('');
  const [repeat, setRepeat] = useState('');
  const [aware, setAware] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const name = folder.slice(folder.lastIndexOf('/') + 1);
  const choosing = mode === 'protect' || mode === 'password';
  const chosen = mode === 'protect' ? password : next;
  const problem = choosing && chosen.length < MIN_PASSWORD ? `Use pelo menos ${MIN_PASSWORD} caracteres.`
    : choosing && chosen !== repeat ? 'As senhas não conferem.'
    : mode === 'protect' && !aware ? 'Confirme que entendeu os avisos.' : '';
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || !password || problem) return;
    setBusy(true); setError('');
    try { await onSubmit(password, next); } catch (e) { setError(errorText(e)); setBusy(false); }
  }
  return <form className="lock-form" onSubmit={e => void submit(e)}>
    {mode === 'protect' && <>
      <p>Os arquivos de <b>{name}</b> (notas, imagens e PDFs) vão ficar cifrados no disco{git ? ' e no GitHub' : ''}. Só a Athena os mostra, depois da senha; a Iris e a conversa com a IA não os veem.</p>
      <ul className="lock-warnings">
        <li><b>Sem a senha, não há como recuperar os arquivos.</b></li>
        <li>O Obsidian deixa de abrir o que estiver nesta pasta.</li>
        {git && <li>Versões antigas que já estão no histórico do Git continuam legíveis lá.</li>}
        <li>Cada arquivo pode ter até 100 MB.</li>
      </ul>
    </>}
    {mode === 'unlock' && <p>Digite a senha de <b>{name}</b>. Ela fica aberta por 10 minutos; depois disso, bloqueia quando você sair dela.</p>}
    {mode === 'unprotect' && <p>Os arquivos de <b>{name}</b> voltam a ser comuns, legíveis no disco, no Obsidian e no Git.</p>}
    <label>{mode === 'protect' ? 'Senha' : mode === 'password' ? 'Senha atual' : 'Senha'}
      <input type="password" autoFocus={autoFocus} autoComplete={mode === 'protect' ? 'new-password' : 'current-password'} value={password} onChange={e => setPassword(e.target.value)} /></label>
    {mode === 'password' && <label>Nova senha<input type="password" autoComplete="new-password" value={next} onChange={e => setNext(e.target.value)} /></label>}
    {choosing && <label>Repita a {mode === 'password' ? 'nova ' : ''}senha<input type="password" autoComplete="new-password" value={repeat} onChange={e => setRepeat(e.target.value)} /></label>}
    {mode === 'protect' && <label className="lock-aware"><input type="checkbox" checked={aware} onChange={e => setAware(e.target.checked)} /> Entendo que, sem a senha, os arquivos não podem ser recuperados.</label>}
    {(error || (password && problem)) && <p role="alert" className="lock-error">{error || problem}</p>}
    <div className="row-actions">
      <button type="submit" className={mode === 'unprotect' ? 'danger-action' : 'primary-action'} disabled={busy || !password || Boolean(problem)}>
        {busy ? 'Aguarde…' : mode === 'protect' ? 'Proteger' : mode === 'unlock' ? 'Abrir' : mode === 'password' ? 'Trocar a senha' : 'Tirar a proteção'}
      </button>
      {onCancel && <button type="button" className="ghost-button" onClick={onCancel}>Cancelar</button>}
    </div>
  </form>;
}

export function LockDialog(props: { mode: LockMode; folder: string; git?: boolean; onSubmit: (password: string, next: string) => Promise<void>; onClose: () => void }) {
  return <div className="dialog-backdrop" onClick={props.onClose}>
    <div className="dialog lock-dialog" role="dialog" aria-modal="true" aria-labelledby="lock-title" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape') props.onClose(); }}>
      <h2 id="lock-title"><LockIcon size={18} /> {titles[props.mode]}</h2>
      <LockForm mode={props.mode} folder={props.folder} git={props.git} onSubmit={props.onSubmit} onCancel={props.onClose} />
    </div>
  </div>;
}

/** Aba de uma nota ou arquivo de pasta bloqueada: pede a senha no lugar do conteúdo. */
export function LockedView({ folder, onUnlock }: { folder: string; onUnlock: (password: string) => Promise<void> }) {
  return <div className="note-scroll"><div className="locked-view">
    <LockIcon size={32} />
    <h1>{folder.slice(folder.lastIndexOf('/') + 1)}</h1>
    <LockForm mode="unlock" folder={folder} onSubmit={password => onUnlock(password)} />
  </div></div>;
}

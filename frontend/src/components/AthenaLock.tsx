import { useState, type FormEvent } from 'react';
import { LockIcon } from './Icons';
import { errorText } from '../services/Desktop';

/** O que o diálogo faz com a pasta ou o arquivo: proteger, abrir, trocar a senha, tirar a proteção
 * ou recuperar com a chave de recuperação global (senha esquecida). */
export type LockMode = 'protect' | 'unlock' | 'password' | 'unprotect' | 'recover';
export const MIN_PASSWORD = 8;
/** Depois da senha, a pasta fica livre por 10 minutos; passado isso, bloqueia ao sair dela. */
export const UNLOCK_GRACE_MS = 10 * 60_000;

const titles: Record<LockMode, string> = {
  protect: 'Proteger com senha', unlock: 'Pasta protegida', password: 'Trocar a senha', unprotect: 'Tirar a proteção',
  recover: 'Esqueci a senha',
};
const actions: Record<LockMode, string> = {
  protect: 'Proteger', unlock: 'Abrir', password: 'Trocar a senha', unprotect: 'Tirar a proteção',
  recover: 'Definir a senha nova',
};

/** Resposta de um envio: a chave de recuperação, quando uma acabou de ser criada. */
export type LockResult = string | void;

/** Formulário de senha das pastas protegidas da Athena (num diálogo ou dentro de uma aba). */
function LockForm({ mode, folder, file = false, git, recovery, onSubmit, onCancel, onForgot, autoFocus = true }: {
  mode: LockMode; folder: string; file?: boolean; git?: boolean; recovery?: boolean; autoFocus?: boolean;
  onSubmit: (secret: string, next: string) => Promise<LockResult>; onCancel?: () => void; onForgot?: () => void;
}) {
  const [secret, setSecret] = useState('');
  const [next, setNext] = useState('');
  const [repeat, setRepeat] = useState('');
  const [aware, setAware] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const name = folder.slice(folder.lastIndexOf('/') + 1);
  const choosing = mode === 'protect' || mode === 'password' || mode === 'recover';
  const chosen = mode === 'protect' ? secret : next;
  const problem = choosing && chosen.length < MIN_PASSWORD ? `Use pelo menos ${MIN_PASSWORD} caracteres.`
    : choosing && chosen !== repeat ? 'As senhas não conferem.'
    : mode === 'protect' && !aware ? 'Confirme que entendeu os avisos.' : '';
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || !secret || problem) return;
    setBusy(true); setError('');
    try { await onSubmit(secret, next); } catch (e) { setError(errorText(e)); setBusy(false); }
  }
  return <form className="lock-form" onSubmit={e => void submit(e)}>
    {mode === 'protect' && <>
      {file
        ? <p>O conteúdo de <b>{name}</b> vai ficar cifrado no disco{git ? ' e no GitHub' : ''}; o nome continua visível. Só a Athena o mostra, depois da senha; a Iris e a conversa com a IA não o veem.</p>
        : <p>Os arquivos de <b>{name}</b> (notas, imagens, PDFs e livros) vão ficar cifrados no disco{git ? ' e no GitHub' : ''}. Só a Athena os mostra, depois da senha; a Iris e a conversa com a IA não os veem.</p>}
      <ul className="lock-warnings">
        <li>Se você esquecer a senha, só a <b>chave de recuperação global</b> da Athena abre de novo. Se ela ainda não existir, a Athena a mostra em seguida, uma única vez.</li>
        <li>O Obsidian deixa de abrir {file ? 'este arquivo' : 'o que estiver nesta pasta'}.</li>
        {git && <li>Versões antigas que já estão no histórico do Git continuam legíveis lá.</li>}
        <li>Cada arquivo pode ter até 100 MB.</li>
      </ul>
    </>}
    {mode === 'unlock' && <p>Digite a senha de <b>{name}</b>. {file ? 'O arquivo fica aberto' : 'A pasta fica aberta'} por 10 minutos; depois disso, bloqueia quando você sair {file ? 'dele' : 'dela'}.</p>}
    {mode === 'unprotect' && <p>{file ? <><b>{name}</b> volta a ser um arquivo comum</> : <>Os arquivos de <b>{name}</b> voltam a ser comuns</>}, legível no disco, no Obsidian e no Git.</p>}
    {mode === 'recover' && <p>Digite a chave de recuperação global da Athena (a que começa com ATHENA-) e escolha uma senha nova para <b>{name}</b>. A chave continua valendo depois.</p>}
    {mode === 'recover'
      ? <label>Chave de recuperação<input className="lock-code-input" autoFocus={autoFocus} autoComplete="off" spellCheck={false} autoCapitalize="characters" placeholder="ATHENA-XXXX-XXXX-…" value={secret} onChange={e => setSecret(e.target.value)} /></label>
      : <label>{mode === 'password' ? 'Senha atual' : 'Senha'}
          <input type="password" autoFocus={autoFocus} autoComplete={mode === 'protect' ? 'new-password' : 'current-password'} value={secret} onChange={e => setSecret(e.target.value)} /></label>}
    {(mode === 'password' || mode === 'recover') && <label>Nova senha<input type="password" autoComplete="new-password" value={next} onChange={e => setNext(e.target.value)} /></label>}
    {choosing && <label>Repita a {mode === 'protect' ? '' : 'nova '}senha<input type="password" autoComplete="new-password" value={repeat} onChange={e => setRepeat(e.target.value)} /></label>}
    {mode === 'protect' && <label className="lock-aware"><input type="checkbox" checked={aware} onChange={e => setAware(e.target.checked)} /> Entendo que, sem a senha e sem a chave de recuperação, os arquivos não podem ser recuperados.</label>}
    {(error || (secret && problem)) && <p role="alert" className="lock-error">{error || problem}</p>}
    <div className="row-actions">
      <button type="submit" className={mode === 'unprotect' ? 'danger-action' : 'primary-action'} disabled={busy || !secret || Boolean(problem)}>{busy ? 'Aguarde…' : actions[mode]}</button>
      {onCancel && <button type="button" className="ghost-button" onClick={onCancel}>Cancelar</button>}
    </div>
    {mode === 'unlock' && onForgot && (recovery
      ? <button type="button" className="link-button lock-forgot" onClick={onForgot}>Esqueci a senha</button>
      : <p className="lock-hint">Este item ainda não tem chave de recuperação: ele ganha a global quando for aberto com a senha.</p>)}
  </form>;
}

/** Chave de recuperação recém-criada: mostrada uma única vez, só fecha depois de confirmar. */
export function RecoveryCode({ code, onDone }: { code: string; onDone: () => void }) {
  const [saved, setSaved] = useState(false);
  const [copied, setCopied] = useState(false);
  return <div className="lock-form">
    <p>Esta é a <b>chave de recuperação global</b> da Athena: com ela você define uma senha nova para qualquer pasta ou arquivo protegido, em qualquer computador. Guarde fora deste computador (papel, gerenciador de senhas). <b>Ela não será mostrada de novo</b>, e quem a tiver consegue abrir todos os itens protegidos.</p>
    <code className="lock-code" aria-label="Chave de recuperação">{code}</code>
    <div className="row-actions">
      <button type="button" className="ghost-button" onClick={() => void navigator.clipboard.writeText(code).then(() => setCopied(true), () => setCopied(false))}>{copied ? 'Copiada' : 'Copiar'}</button>
    </div>
    <label className="lock-aware"><input type="checkbox" checked={saved} onChange={e => setSaved(e.target.checked)} /> Guardei a chave de recuperação.</label>
    <div className="row-actions"><button type="button" className="primary-action" disabled={!saved} onClick={onDone}>Concluir</button></div>
  </div>;
}

export function LockDialog(props: {
  mode: LockMode; folder: string; file?: boolean; git?: boolean; recovery?: boolean;
  onSubmit: (mode: LockMode, secret: string, next: string) => Promise<LockResult>; onClose: () => void;
}) {
  const [mode, setMode] = useState(props.mode);
  const [code, setCode] = useState<string | null>(null);
  // Com a chave na tela, fechar por fora não vale: só "Concluir", depois de confirmar que guardou.
  const close = () => { if (!code) props.onClose(); };
  return <div className="dialog-backdrop" onClick={close}>
    <div className="dialog lock-dialog" role="dialog" aria-modal="true" aria-labelledby="lock-title" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape') close(); }}>
      <h2 id="lock-title"><LockIcon size={18} /> {code ? 'Chave de recuperação' : titles[mode]}</h2>
      {code
        ? <RecoveryCode code={code} onDone={props.onClose} />
        : <LockForm key={mode} mode={mode} folder={props.folder} file={props.file} git={props.git} recovery={props.recovery} onCancel={props.onClose} onForgot={() => setMode('recover')}
            onSubmit={async (secret, next) => { const result = await props.onSubmit(mode, secret, next); if (result) setCode(result); else props.onClose(); }} />}
    </div>
  </div>;
}

/** Aba de uma nota ou arquivo de pasta bloqueada: pede a senha no lugar do conteúdo. */
export function LockedView({ folder, file, recovery, onUnlock, onForgot }: { folder: string; file?: boolean; recovery?: boolean; onUnlock: (password: string) => Promise<void>; onForgot: () => void }) {
  return <div className="note-scroll"><div className="locked-view">
    <LockIcon size={32} />
    <h1>{folder.slice(folder.lastIndexOf('/') + 1)}</h1>
    <LockForm mode="unlock" folder={folder} file={file} recovery={recovery} onSubmit={password => onUnlock(password)} onForgot={onForgot} />
  </div></div>;
}

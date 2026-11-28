import { useCallback, useEffect, useState } from 'react';
import Confirm from './Confirm';
import { CloseIcon, FileIcon, RefreshIcon, WarningIcon } from './Icons';
import { errorText, native } from '../services/Desktop';

type Mudanca = { caminho: string; estado: 'modificado' | 'novo' | 'apagado' | 'renomeado' | 'fora'; explicacao: string | null; sensivel: string | null };
type Estado = { pasta: string; ramo: string; remoto: string | null; endereco: string | null; a_enviar: number; a_trazer: number; mudancas: Mudanca[] };
type Resultado = { commit: boolean; trazidos: number; enviados: number };
type Conteudo = { texto: string; diferenca: string | null; binario: boolean; cortado: boolean };

/** Grupos na ordem em que aparecem (arquivo fora do git = novo). */
const GROUPS: { title: string; has: (e: Mudanca['estado']) => boolean }[] = [
  { title: 'Mudaram', has: e => e === 'modificado' || e === 'renomeado' },
  { title: 'Novos', has: e => e === 'novo' || e === 'fora' },
  { title: 'Apagados', has: e => e === 'apagado' },
];
const hoje = () => new Date().toLocaleDateString('pt-BR');
const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** Dotfiles: o que vai para o repositório (o que mudou, os novos e os apagados) e um botão para
 * sincronizar com o remoto (SSH). Clicar num arquivo mostra o texto; o hover explica o que ele faz. */
export default function HandDotfiles({ active }: { active: boolean }) {
  const [estado, setEstado] = useState<Estado | null>(null);
  const [message, setMessage] = useState(`Atualização pela Ayla em ${hoje()}`);
  const [viewing, setViewing] = useState<Mudanca | null>(null);
  const [busy, setBusy] = useState(false);
  const [explaining, setExplaining] = useState(false);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const load = useCallback(() => native<Estado>('hand_dotfiles_estado').then(e => { setEstado(e); setError(''); return e; }).catch(e => { setError(errorText(e)); return null; }), []);
  // Ao entrar: a lista na hora; depois a Ayla explica os arquivos que ainda não conhece.
  useEffect(() => {
    if (!active) return;
    let alive = true;
    void load().then(e => {
      if (!e || !e.mudancas.some(m => !m.explicacao)) return;
      setExplaining(true);
      void native<number>('hand_dotfiles_explicar').then(n => { if (alive && n) void load(); }).catch(() => {}).finally(() => { if (alive) setExplaining(false); });
    });
    return () => { alive = false; };
  }, [active, load]);
  async function sync() {
    if (!estado) return;
    setBusy(true); setNotice(''); setError('');
    try {
      const r = await native<Resultado>('hand_dotfiles_sincronizar', { arquivos: estado.mudancas.map(m => m.caminho), mensagem: message });
      const parts = [r.commit ? `commit de ${plural(estado.mudancas.length, 'arquivo', 'arquivos')}` : '', r.trazidos ? `${plural(r.trazidos, 'mudança trazida', 'mudanças trazidas')} do remoto` : '', r.enviados ? plural(r.enviados, 'commit enviado', 'commits enviados') : ''].filter(Boolean);
      setNotice(parts.length ? `Sincronizado: ${parts.join(', ')}.` : 'Já estava tudo igual ao remoto.');
      setMessage(`Atualização pela Ayla em ${hoje()}`);
    } catch (e) { setError(errorText(e)); } finally { await load(); setBusy(false); }
  }
  if (!estado) return <div className="hand-panel">{error ? <p className="app-error">{error}</p> : <p role="status" className="panel-muted">Lendo o repositório…</p>}</div>;
  const remote = estado.remoto ?? 'sem remoto';
  const sensitive = estado.mudancas.filter(m => m.sensivel);
  const total = estado.mudancas.length;
  return <div className="hand-panel">
    <div className="hand-search">
      <code className="hand-repo">{estado.pasta.replace(/^\/home\/[^/]+/, '~')} · {estado.ramo} → {remote}</code>
      <button className="icon-button" aria-label="Atualizar" data-tip="Atualizar" onClick={() => void load()}><RefreshIcon size={16} /></button>
    </div>
    <p className="panel-muted">Sincronizar envia tudo o que está abaixo para {estado.endereco ?? 'o remoto'} (pela sua chave SSH) e traz o que mudou lá.{estado.a_enviar || estado.a_trazer ? ` Pelo último contato: ${estado.a_enviar} para enviar, ${estado.a_trazer} para trazer.` : ''}</p>
    {explaining && <p role="status" className="hand-progress">A Ayla está lendo os arquivos para explicar cada um…</p>}
    {total === 0 ? <p className="panel-muted">Nada mudou aqui desde o último commit.</p>
      : GROUPS.map(group => {
        const items = estado.mudancas.filter(m => group.has(m.estado));
        if (!items.length) return null;
        return <section key={group.title} className="hand-dot-group">
          <h3>{group.title} <small>{items.length}</small></h3>
          <ul className="hand-dot-files">{items.map(m => <li key={m.caminho}>
            <button className={`hand-dot-file ${m.estado}`} aria-describedby={`tip-${m.caminho}`} onClick={() => setViewing(m)}>
              {m.sensivel ? <span className="hand-dot-warn" aria-label="Atenção"><WarningIcon size={14} /></span> : <FileIcon size={14} />}
              <span className="hand-dot-path">{m.caminho}</span>
            </button>
            <div role="tooltip" id={`tip-${m.caminho}`} className="hand-dot-tip">
              <p>{m.explicacao ?? (explaining ? 'A Ayla ainda está lendo este arquivo…' : 'Sem explicação (precisa do modelo local).')}</p>
              {m.sensivel && <p className="hand-dot-tip-warn">Atenção: {m.sensivel}. Confira antes de enviar.</p>}
            </div>
          </li>)}</ul>
        </section>;
      })}
    {total > 0 && <input value={message} aria-label="Mensagem do commit" placeholder="Mensagem do commit" onChange={e => setMessage(e.target.value)} />}
    <div className="row-actions">
      <Confirm className="primary-action" disabled={busy || !estado.remoto || (total > 0 && !message.trim())}
        question={total ? `Enviar ${plural(total, 'arquivo', 'arquivos')} para ${remote}${sensitive.length ? ` (${plural(sensitive.length, 'parece ter', 'parecem ter')} senha ou chave)` : ''} e trazer o que mudou lá?` : `Trazer o que mudou em ${remote}?`}
        confirm="Sincronizar" onConfirm={() => void sync()}>{busy ? 'Sincronizando…' : 'Sincronizar'}</Confirm>
    </div>
    {notice && <p role="status" className="hand-done">{notice}</p>}
    {error && <p role="alert" className="app-error">{error}</p>}
    {viewing && <FileModal file={viewing} close={() => setViewing(null)} />}
  </div>;
}

function FileModal({ file, close }: { file: Mudanca; close: () => void }) {
  const [content, setContent] = useState<Conteudo | null>(null);
  const [error, setError] = useState('');
  const [tab, setTab] = useState<'arquivo' | 'mudou'>('arquivo');
  useEffect(() => { void native<Conteudo>('hand_dotfiles_conteudo', { caminho: file.caminho }).then(setContent).catch(e => setError(errorText(e))); }, [file.caminho]);
  const label = file.estado === 'apagado' ? 'Apagado (como estava no último commit)' : file.estado === 'fora' || file.estado === 'novo' ? 'Novo' : 'Mudou';
  return <div className="dialog-backdrop" onClick={close}>
    <div className="dialog hand-file-dialog" role="dialog" aria-modal="true" aria-labelledby="hand-file-title" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape') close(); }}>
      <header className="hand-file-head">
        <div><h2 id="hand-file-title">{file.caminho}</h2><small>{label}{file.explicacao ? ` · ${file.explicacao}` : ''}</small></div>
        <button className="icon-button" autoFocus aria-label="Fechar" onClick={close}><CloseIcon size={16} /></button>
      </header>
      {file.sensivel && <p className="app-error">Atenção: {file.sensivel}.</p>}
      {content?.diferenca && <div className="hand-tabs" role="tablist">
        <button role="tab" aria-selected={tab === 'arquivo'} className={tab === 'arquivo' ? 'hand-tab on' : 'hand-tab'} onClick={() => setTab('arquivo')}>Arquivo</button>
        <button role="tab" aria-selected={tab === 'mudou'} className={tab === 'mudou' ? 'hand-tab on' : 'hand-tab'} onClick={() => setTab('mudou')}>O que mudou</button>
      </div>}
      {error && <p role="alert" className="app-error">{error}</p>}
      {!content && !error && <p role="status" className="panel-muted">Abrindo…</p>}
      {content && (content.binario ? <p className="panel-muted">Arquivo binário: não dá para mostrar como texto.</p>
        : tab === 'mudou' && content.diferenca
          ? <pre className="hand-diff hand-file-text">{content.diferenca.split('\n').map((line, i) => <span key={i} className={line.startsWith('+') && !line.startsWith('+++') ? 'add' : line.startsWith('-') && !line.startsWith('---') ? 'del' : undefined}>{line}{'\n'}</span>)}</pre>
          : <pre className="hand-diff hand-file-text">{content.texto || '(vazio)'}</pre>)}
      {content?.cortado && <small className="panel-muted">Mostrando os primeiros 200 KB.</small>}
    </div>
  </div>;
}

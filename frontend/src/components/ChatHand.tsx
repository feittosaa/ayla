import { useState } from 'react';
import Confirm from './Confirm';
import { FileIcon } from './Icons';
import { HandPlan } from './Hand';
import { nameOf, short, size, when, type Achado, type HandStep, type Saida } from '../services/Hand';
import { errorText, native } from '../services/Desktop';

/** Cartão do Hand na resposta: captura, busca, plano (com Aplicar), programa aberto ou comando
 * (com Rodar). Tudo neste computador. */
export type HandCard = { home?: string } & (
  | { tipo: 'captura'; caminho: string }
  | { tipo: 'busca'; consulta: string; achados: Achado[] }
  | { tipo: 'plano'; id: string; titulo: string; passos: HandStep[] }
  | { tipo: 'programa'; nome: string; id: string }
  | { tipo: 'comando'; id: string; comando: string });

export default function ChatHand({ item }: { item: HandCard }) {
  const home = item.home ?? '';
  const [error, setError] = useState('');
  const open = (path: string, mostrar = false) => void native('hand_abrir', { caminho: path, mostrar }).catch(e => setError(errorText(e)));
  const body = (() => {
    switch (item.tipo) {
      case 'captura': return <p className="hand-name"><FileIcon size={14} /><b>{nameOf(item.caminho)}</b>
        <span className="row-actions"><button className="ghost-button" onClick={() => open(item.caminho)}>Abrir</button><button className="ghost-button" onClick={() => open(item.caminho, true)}>Mostrar na pasta</button></span></p>;
      case 'busca': return <ul className="hand-list">{item.achados.slice(0, 12).map(a => <li key={a.path}>
        <span className="hand-name" title={a.path}>{a.folder ? '📁' : <FileIcon size={14} />}<b>{a.name}</b><small>{short(a.path.slice(0, a.path.lastIndexOf('/')), home)} · {a.folder ? 'pasta' : size(a.bytes)} · {when(a.modified)}</small></span>
        <span className="row-actions"><button className="ghost-button" onClick={() => open(a.path)}>Abrir</button>{!a.folder && <button className="ghost-button" onClick={() => open(a.path, true)}>Mostrar na pasta</button>}</span>
      </li>)}</ul>;
      case 'plano': return <HandPlan plan={item.passos} title={item.titulo} home={home} origem={item.id} />;
      case 'programa': return null;
      case 'comando': return <HandCommand id={item.id} command={item.comando} />;
    }
  })();
  if (!body) return null;
  return <section className="chat-hand" aria-label="Hand">{body}{error && <p role="alert" className="app-error">{error}</p>}</section>;
}

function HandCommand({ id, command }: { id: string; command: string }) {
  const key = `ayla-hand-comando-${id}`;
  const [out, setOut] = useState<Saida | null>(() => { try { return JSON.parse(localStorage.getItem(key) ?? 'null') as Saida | null; } catch { return null; } });
  const [running, setRunning] = useState(false);
  const [error, setError] = useState('');
  async function run() {
    setRunning(true); setError('');
    try { const result = await native<Saida>('hand_comando', { comando: command }); setOut(result); localStorage.setItem(key, JSON.stringify({ ...result, stdout: result.stdout.slice(0, 4000), stderr: result.stderr.slice(0, 2000) })); }
    catch (e) { setError(errorText(e)); } finally { setRunning(false); }
  }
  return <div className="hand-run">
    <code>$ {command}</code>
    {!out && <div className="row-actions"><Confirm className="primary-action" disabled={running} question="Rodar este comando no seu computador?" confirm="Rodar" onConfirm={() => void run()}>{running ? 'Rodando…' : 'Rodar'}</Confirm></div>}
    {out && <>
      {out.stdout && <pre>{out.stdout}</pre>}
      {out.stderr && <pre className="hand-stderr">{out.stderr}</pre>}
      <small className="panel-muted">{out.tempo_esgotado ? 'Parado: passou de 60 s.' : `Terminou com código ${out.code ?? '?'}.`}{out.cortado ? ' Saída cortada.' : ''}</small>
    </>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </div>;
}

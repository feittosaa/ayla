import { useState } from 'react';
import { FileIcon, FolderIcon } from './Icons';
import { HandPlan } from './HandPlan';
import HandCommand from './HandCommand';
import HandTask, { type HandTaskCard } from './HandTask';
import HandWallpaper, { type HandWallpaperCard } from './HandWallpaper';
import { nameOf, short, size, when, type Achado, type HandStep } from '../services/Hand';
import { errorText, native } from '../services/Desktop';

/** Cartão do Hand na resposta: captura, busca, plano (com Aplicar), programa aberto ou comando
 * (com Rodar). Tudo neste computador. */
export type HandCard = { home?: string } & (
  | { tipo: 'captura'; caminho: string }
  | { tipo: 'busca'; consulta: string; achados: Achado[] }
  | { tipo: 'plano'; id: string; titulo: string; passos: HandStep[] }
  | { tipo: 'programa'; nome: string; id: string }
  | { tipo: 'comando'; id: string; comando: string }
  | ({ tipo: 'tarefa' } & HandTaskCard)
  | ({ tipo: 'papel_de_parede' } & HandWallpaperCard)
  // Pergunta esperando resposta ("pela Ayla ou pelo navegador?"): o texto já diz tudo.
  | { tipo: 'pergunta'; objetivo: string }
  // Rotina montada pela conversa: chega desligada; "Ativar" liga (se não faltar nada).
  | { tipo: 'rotina'; id: string; nome: string; resumo: string; falta: string | null });

export default function ChatHand({ item }: { item: HandCard }) {
  const home = item.home ?? '';
  const [error, setError] = useState('');
  const open = (path: string, mostrar = false) => void native('hand_abrir', { caminho: path, mostrar }).catch(e => setError(errorText(e)));
  const body = (() => {
    switch (item.tipo) {
      case 'captura': return <p className="hand-name"><FileIcon size={14} /><b>{nameOf(item.caminho)}</b>
        <span className="row-actions"><button className="ghost-button" onClick={() => open(item.caminho)}>Abrir</button><button className="ghost-button" onClick={() => open(item.caminho, true)}>Mostrar na pasta</button></span></p>;
      case 'busca': return <ul className="hand-list">{item.achados.slice(0, 12).map(a => <li key={a.path}>
        <span className="hand-name" title={a.path}>{a.folder ? <span className="hand-folder-icon"><FolderIcon size={15} /></span> : <FileIcon size={14} />}<b>{a.name}</b><small>{short(a.path.slice(0, a.path.lastIndexOf('/')), home)} · {a.folder ? 'pasta' : size(a.bytes)} · {when(a.modified)}</small></span>
        <span className="row-actions"><button className="ghost-button" onClick={() => open(a.path)}>Abrir</button>{!a.folder && <button className="ghost-button" onClick={() => open(a.path, true)}>Mostrar na pasta</button>}</span>
      </li>)}</ul>;
      case 'plano': return <HandPlan plan={item.passos} title={item.titulo} home={home} origem={item.id} />;
      case 'programa': return null;
      case 'comando': return <HandCommand id={item.id} command={item.comando} />;
      case 'tarefa': return <HandTask task={item} />;
      case 'papel_de_parede': return <HandWallpaper card={item} />;
      case 'pergunta': return null;
      case 'rotina': return <RoutineCard id={item.id} nome={item.nome} resumo={item.resumo} falta={item.falta} />;
    }
  })();
  if (!body) return null;
  return <section className="chat-hand" aria-label="Hand">{body}{error && <p role="alert" className="app-error">{error}</p>}</section>;
}

function RoutineCard({ id, nome, resumo, falta }: { id: string; nome: string; resumo: string; falta: string | null }) {
  const key = `ayla-rotina-ativada-${id}`;
  const [on, setOn] = useState(() => !!localStorage.getItem(key));
  const [error, setError] = useState('');
  async function activate() {
    try { await native('rotina_ativar', { id, ativa: true }); localStorage.setItem(key, '1'); setOn(true); } catch (e) { setError(errorText(e)); }
  }
  return <div className="hand-plan">
    <header><b>{nome}</b><small>{resumo}</small></header>
    {falta ? <p className="panel-muted">Falta: {falta}. Ajuste em Hand › Rotinas.</p>
      : on ? <p role="status" className="hand-done">Ativada. Dá para pausar, editar e ver o histórico em Hand › Rotinas.</p>
      : <div className="row-actions"><button className="primary-action" onClick={() => void activate()}>Ativar</button><span className="panel-muted">ou ajuste em Hand › Rotinas</span></div>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </div>;
}

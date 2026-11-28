import { useRef, useState } from 'react';
import { Channel } from '@tauri-apps/api/core';
import { errorText, native } from '../services/Desktop';

/** Tarefa do Hand: a Ayla opera o computador olhando a tela. Você aprova o plano uma vez; ela
 * segue sozinha e para para perguntar antes de algo irreversível. Parar: atalho ou botão. */
export type HandTaskCard = { id: string; objetivo: string; passos: string[]; precisa?: string };
type Fim = { estado: 'concluida' | 'parada' | 'limite' | 'pergunta'; mensagem: string; acao?: string; so_pergunta?: boolean };
type Event = { type: 'passo'; texto: string } | { type: 'olhando' } | { type: 'comecou'; atalho?: string | null };

export default function HandTask({ task }: { task: HandTaskCard }) {
  const key = `ayla-hand-tarefa-${task.id}`;
  const saved = (() => { try { return JSON.parse(localStorage.getItem(key) ?? 'null') as { feitos: string[]; fim: Fim | null } | null; } catch { return null; } })();
  const [state, setState] = useState<'plano' | 'rodando' | 'pergunta' | 'fim'>(saved?.fim ? 'fim' : 'plano');
  const [feitos, setFeitos] = useState<string[]>(saved?.feitos ?? []);
  const [fim, setFim] = useState<Fim | null>(saved?.fim ?? null);
  const [olhando, setOlhando] = useState(false);
  const [atalho, setAtalho] = useState<string | null>(null);
  const [error, setError] = useState('');
  const done = useRef<string[]>(saved?.feitos ?? []);
  const keep = (f: Fim | null) => localStorage.setItem(key, JSON.stringify({ feitos: done.current.slice(-40), fim: f }));
  async function go(command: 'hand_tarefa_executar' | 'hand_tarefa_responder', extra: Record<string, unknown> = {}) {
    setState('rodando'); setError(''); setFim(null);
    const events = new Channel<Event>();
    events.onmessage = event => {
      if (event.type === 'passo') { done.current = [...done.current, event.texto]; setFeitos(done.current); setOlhando(false); }
      if (event.type === 'olhando') setOlhando(true);
      if (event.type === 'comecou') setAtalho(event.atalho ?? null);
    };
    try {
      const result = await native<Fim>(command, { id: task.id, events, ...extra });
      setFim(result); setState(result.estado === 'pergunta' ? 'pergunta' : 'fim');
      keep(result.estado === 'pergunta' ? null : result);
    } catch (e) { setError(errorText(e)); setState('fim'); keep({ estado: 'parada', mensagem: errorText(e) }); }
    finally { setOlhando(false); }
  }
  return <section className="hand-plan hand-task" aria-label="Tarefa no computador">
    <header><b>{task.objetivo}</b><small>Plano · {task.passos.length} {task.passos.length === 1 ? 'passo' : 'passos'}</small></header>
    <ol className="hand-task-plan">{task.passos.map((p, i) => <li key={i}>{p}</li>)}</ol>
    {task.precisa && <p className="panel-muted">Vai precisar de você: {task.precisa}</p>}
    {feitos.length > 0 && <ul className="hand-steps hand-task-done" aria-label="Feito">{feitos.map((f, i) => <li key={i}><span className="hand-verb">{i + 1}.</span><span>{f}</span></li>)}</ul>}
    {state === 'plano' && <>
      <p className="panel-muted">Ao aprovar, a Ayla usa o seu teclado e mouse sozinha, olhando a tela: enquanto ela opera, deixe-os com ela. Ela para para perguntar antes de enviar, comprar, apagar ou digitar senha.</p>
      <div className="row-actions"><button className="primary-action" onClick={() => void go('hand_tarefa_executar')}>Aprovar e começar</button>
        <button className="ghost-button" onClick={() => { setState('fim'); const f: Fim = { estado: 'parada', mensagem: 'Cancelada; nada foi feito.' }; setFim(f); keep(f); }}>Cancelar</button></div>
    </>}
    {state === 'rodando' && <div className="row-actions hand-task-running">
      <span role="status">{olhando ? 'Olhando a tela…' : 'Fazendo…'}{atalho ? ` · ${atalho} para parar` : ''}</span>
      <button className="danger-action" onClick={() => void native('hand_tarefa_parar').catch(() => undefined)}>Parar</button>
    </div>}
    {state === 'pergunta' && fim && <div className="hand-task-question">
      <p><b>{fim.mensagem}</b></p>
      <div className="row-actions">
        <button className="primary-action" onClick={() => void go('hand_tarefa_responder', { aprovar: true })}>{fim.so_pergunta ? 'Já resolvi, pode seguir' : 'Pode fazer'}</button>
        <button className="ghost-button" onClick={() => void go('hand_tarefa_responder', { aprovar: false })}>Parar aqui</button>
      </div>
    </div>}
    {state === 'fim' && fim && <p role="status" className={fim.estado === 'concluida' ? 'hand-done' : 'panel-muted'}>{fim.mensagem}</p>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

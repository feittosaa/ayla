import { useState } from 'react';
import Confirm from './Confirm';
import { type Saida } from '../services/Hand';
import { errorText, native } from '../services/Desktop';

/** Comando proposto (pela conversa ou pelo plano de uma tarefa): só roda com Rodar e a confirmação. */
export default function HandCommand({ id, command }: { id: string; command: string }) {
  const key = `ayla-hand-comando-${id}`;
  const [out, setOut] = useState<Saida | null>(() => { try { return JSON.parse(localStorage.getItem(key) ?? 'null') as Saida | null; } catch { return null; } });
  const [running, setRunning] = useState(false);
  const [cancelled, setCancelled] = useState(() => localStorage.getItem(`${key}-cancelado`) === '1');
  const [error, setError] = useState('');
  async function run() {
    setRunning(true); setError('');
    try { const result = await native<Saida>('hand_comando', { comando: command }); setOut(result); localStorage.setItem(key, JSON.stringify({ ...result, stdout: result.stdout.slice(0, 4000), stderr: result.stderr.slice(0, 2000) })); }
    catch (e) { setError(errorText(e)); } finally { setRunning(false); }
  }
  return <div className="hand-run">
    <code>$ {command}</code>
    {!out && !cancelled && <div className="row-actions"><Confirm className="primary-action" disabled={running} question="Rodar este comando no seu computador?" confirm="Rodar" onConfirm={() => void run()}>{running ? 'Rodando…' : 'Rodar'}</Confirm>
      <button type="button" className="ghost-button" disabled={running} onClick={() => { setCancelled(true); localStorage.setItem(`${key}-cancelado`, '1'); }}>Cancelar</button></div>}
    {!out && cancelled && <small className="panel-muted">Cancelado: o comando não rodou.</small>}
    {out && <>
      {out.stdout && <pre>{out.stdout}</pre>}
      {out.stderr && <pre className="hand-stderr">{out.stderr}</pre>}
      <small className="panel-muted">{out.tempo_esgotado ? 'Parado: passou de 60 s.' : `Terminou com código ${out.code ?? '?'}.`}{out.cortado ? ' Saída cortada.' : ''}</small>
    </>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </div>;
}

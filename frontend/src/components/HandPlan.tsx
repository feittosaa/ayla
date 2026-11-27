import { useEffect, useState } from 'react';
import { errorText, native } from '../services/Desktop';
import { baseOf, nameOf, short, type HandStep, type Lote, type Resultado } from '../services/Hand';

/** Uma linha do plano, em português: "Mover foto.png → Imagens/foto.png" (relativo à pasta arrumada). */
export function StepLine({ step, home, base }: { step: HandStep; home: string; base?: string }) {
  const show = (path: string) => base && path.startsWith(`${base}/`) ? path.slice(base.length + 1) : short(path, home);
  if (step.acao === 'pasta') return <><span className="hand-verb">Criar pasta</span><span>{show(step.caminho)}</span></>;
  if (step.acao === 'lixeira') return <><span className="hand-verb danger">Lixeira</span><span>{show(step.caminho)}</span></>;
  if (step.acao === 'iris') return <><span className="hand-verb">Da Iris</span><span>{step.nome} → {show(step.para.slice(0, step.para.lastIndexOf('/')))}{nameOf(step.para) !== step.nome ? ` (como ${nameOf(step.para)})` : ''}</span></>;
  const sameDir = step.de.slice(0, step.de.lastIndexOf('/')) === step.para.slice(0, step.para.lastIndexOf('/'));
  return <><span className="hand-verb">{sameDir ? 'Renomear' : 'Mover'}</span><span>{show(step.de)} → {sameDir ? nameOf(step.para) : show(step.para)}</span></>;
}

/** Plano mostrado antes de qualquer mudança: só com Aplicar. Usado na página e na conversa. */
export function HandPlan({ plan, title, home, origem, onDone }: { plan: HandStep[]; title: string; home: string; origem?: string; onDone?: (result: Resultado) => void }) {
  const cancelKey = origem ? `ayla-hand-cancelado-${origem}` : '';
  const [state, setState] = useState<'idle' | 'applying' | 'done' | 'cancelled'>(() => cancelKey && localStorage.getItem(cancelKey) ? 'cancelled' : 'idle');
  const [result, setResult] = useState<Resultado | null>(null);
  // Cartão da conversa já aplicado: volta assim ao reabrir (pelo histórico do Hand).
  useEffect(() => {
    if (!origem) return;
    let alive = true;
    void native<Lote[]>('hand_historico').then(lotes => { const lote = lotes.find(l => l.origem === origem); if (alive && lote) { setResult({ lote, falhas: [] }); setState('done'); } }).catch(() => undefined);
    return () => { alive = false; };
  }, [origem]);
  const [error, setError] = useState('');
  const moves = plan.filter(s => s.acao !== 'pasta').length;
  const base = baseOf(plan);
  async function apply() {
    setState('applying'); setError('');
    try { const done = await native<Resultado>('hand_aplicar', { plano: plan, titulo: title, origem: origem ?? null }); setResult(done); setState('done'); onDone?.(done); }
    catch (e) { setError(errorText(e)); setState('idle'); }
  }
  return <section className="hand-plan" aria-label={title}>
    <header><b>{title}</b><small>{base ? `${short(base, home)} · ` : ''}{moves} {moves === 1 ? 'arquivo' : 'arquivos'}{plan.length > moves ? ` · ${plan.length - moves} ${plan.length - moves === 1 ? 'pasta nova' : 'pastas novas'}` : ''}</small></header>
    <ul className="hand-steps">{plan.slice(0, 200).map((step, i) => <li key={i}><StepLine step={step} home={home} base={base} /></li>)}</ul>
    {plan.length > 200 && <p className="panel-muted">E mais {plan.length - 200}.</p>}
    {state === 'done' && result ? <p role="status" className="hand-done">Feito: {result.lote.feitos.filter(s => s.acao !== 'pasta').length} de {moves}. Dá para desfazer em Histórico.{result.falhas.length ? ` ${result.falhas.length} não ${result.falhas.length === 1 ? 'deu' : 'deram'} certo: ${result.falhas.slice(0, 3).map(([s, why]) => `${s.acao === 'mover' ? nameOf(s.de) : s.acao === 'iris' ? s.nome : nameOf(s.caminho)} (${why})`).join('; ')}.` : ''}</p>
      : state === 'cancelled' ? <p role="status">Cancelado; nada mudou.</p>
      : <div className="row-actions">
          <button className="primary-action" disabled={state !== 'idle'} onClick={() => void apply()}>{state === 'applying' ? 'Aplicando…' : 'Aplicar'}</button>
          <button className="ghost-button" disabled={state !== 'idle'} onClick={() => { setState('cancelled'); if (cancelKey) localStorage.setItem(cancelKey, '1'); }}>Cancelar</button>
        </div>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

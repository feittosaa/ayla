import { useState } from 'react';
import { errorText, native } from '../services/Desktop';
import type { FichaCard } from '../services/Ficha';

/** Mudança numa ficha proposta pela Ayla: antes e depois, prévia e Aplicar. Nada é gravado sem o clique. */
export default function ChatFicha({ item }: { item: FichaCard }) {
  const [previews, setPreviews] = useState<string[] | null>(null);
  const [state, setState] = useState<'idle' | 'loading' | 'applying' | 'done' | 'cancelled'>('idle');
  const [error, setError] = useState('');
  const [outside, setOutside] = useState(false);
  async function preview() {
    setState('loading'); setError('');
    try { setPreviews(await native<string[]>('ficha_previa', { root: item.root, path: item.path, primeira: item.primeira, valores: item.valores })); }
    catch (e) { setError(errorText(e)); }
    finally { setState('idle'); }
  }
  async function apply() {
    setState('applying'); setError('');
    try {
      const result = await native<{ mudouFora: boolean }>('ficha_aplicar', { root: item.root, path: item.path, primeira: item.primeira, valores: item.valores });
      setOutside(result.mudouFora); setState('done');
    } catch (e) { setError(errorText(e)); setState('idle'); }
  }
  return <section className="chat-ficha" aria-label={`Ficha de ${item.nome}`}>
    <header><b>Ficha de {item.nome}</b><small>{item.path} · página {item.primeira}</small></header>
    <ul>{item.mudancas.map((m, i) => <li key={i}><span>{m.rotulo}</span>{m.antes ? <><s>{m.antes}</s><span aria-hidden="true">→</span></> : null}<b>{m.depois}</b></li>)}</ul>
    {previews && <div className="chat-ficha-previews">{previews.map((src, i) => <img key={i} src={src} alt={`Prévia da página ${i + 1}`} />)}</div>}
    {state === 'done' ? <p role="status" className="chat-ficha-done">Aplicado no PDF, no mesmo lugar. A versão de antes está em Configurações › Backups{outside ? ' (o PDF tinha mudado fora da Ayla; a versão nova virou a base)' : ''}.</p>
      : state === 'cancelled' ? <p role="status">Cancelado; nada mudou.</p>
      : <div className="row-actions">
          <button className="primary-action" disabled={state !== 'idle'} onClick={() => void apply()}>{state === 'applying' ? 'Aplicando…' : 'Aplicar'}</button>
          {!previews && <button className="ghost-button" disabled={state !== 'idle'} onClick={() => void preview()}>{state === 'loading' ? 'Montando a prévia…' : 'Ver prévia'}</button>}
          <button className="ghost-button" disabled={state !== 'idle'} onClick={() => setState('cancelled')}>Cancelar</button>
        </div>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

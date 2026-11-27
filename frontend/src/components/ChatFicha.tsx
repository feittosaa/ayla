import { useState } from 'react';
import { errorText, native } from '../services/Desktop';
import type { FichaCard } from '../services/Ficha';
import ImageLightbox from './ImageLightbox';
import { FileIcon } from './Icons';

/** Mudança numa ficha proposta pela Ayla: antes e depois, prévia e Aplicar. Nada é gravado sem o clique. */
export default function ChatFicha({ item, conversa, onOpen }: { item: FichaCard; conversa?: string; onOpen?: (path: string) => void }) {
  const feito = item.situacao?.estado === 'aplicado' ? item.situacao : null;
  const [previews, setPreviews] = useState<string[] | null>(null);
  const [state, setState] = useState<'idle' | 'loading' | 'applying' | 'done' | 'cancelled'>(feito ? 'done' : item.situacao?.estado === 'cancelado' ? 'cancelled' : 'idle');
  const [error, setError] = useState('');
  const [outside, setOutside] = useState(feito?.mudouFora ?? false);
  const [renamed, setRenamed] = useState(feito?.renomeado ?? '');
  const [arquivo, setArquivo] = useState(feito?.arquivo ?? '');
  const [zoomed, setZoomed] = useState<number | null>(null);
  async function preview() {
    setState('loading'); setError('');
    try { setPreviews(await native<string[]>('ficha_previa', { root: item.root, path: item.path, primeira: item.primeira, valores: item.valores })); }
    catch (e) { setError(errorText(e)); }
    finally { setState('idle'); }
  }
  async function apply() {
    setState('applying'); setError('');
    try {
      // Pelo chat, o nome novo da ficha também renomeia o PDF (quando ele tinha o nome dela).
      const result = await native<{ mudouFora: boolean; renomeado?: string; arquivo?: string }>('ficha_aplicar', { root: item.root, path: item.path, primeira: item.primeira, valores: item.valores, renomear: true, conversa });
      setOutside(result.mudouFora); setRenamed(result.renomeado ?? ''); setArquivo(result.arquivo ?? ''); setState('done');
    } catch (e) { setError(errorText(e)); setState('idle'); }
  }
  async function cancel() {
    setState('cancelled');
    // Guardado para o cartão voltar cancelado; se falhar, só não fica lembrado.
    if (conversa) await native('ficha_cancelar', { conversa, root: item.root, path: item.path, primeira: item.primeira, valores: item.valores }).catch(() => undefined);
  }
  return <section className="chat-ficha" aria-label={`Ficha de ${item.nome}`}>
    <header><b>Ficha de {item.nome}</b><small>{item.path} · página {item.primeira}</small></header>
    <ul>{item.mudancas.map((m, i) => <li key={i}><span>{m.rotulo}</span>{m.antes ? <><s>{m.antes}</s><span aria-hidden="true">→</span></> : null}<b>{m.depois}</b></li>)}</ul>
    {previews && <div className="chat-ficha-previews">{previews.map((src, i) => <button key={i} type="button" className="chat-ficha-preview" title="Clique para ampliar" onClick={() => setZoomed(i)}><img src={src} alt={`Prévia da página ${i + 1}`} /></button>)}</div>}
    {previews && zoomed !== null && <ImageLightbox paginas={previews} inicio={zoomed} alt={`Prévia da ficha de ${item.nome}`} caption={previews.length > 1 ? `${previews.length} páginas · role para ler` : 'Role para ler'} onClose={() => setZoomed(null)} />}
    {state === 'done' ? <p role="status" className="chat-ficha-done">Aplicado no PDF, no mesmo lugar. A versão de antes está em Configurações › Backups{outside ? ' (o PDF tinha mudado fora da Ayla; a versão nova virou a base)' : ''}.{renamed ? ` O arquivo agora se chama ${renamed.slice(renamed.lastIndexOf('/') + 1)}.` : ''}</p>
      : state === 'cancelled' ? <p role="status">Cancelado; nada mudou.</p>
      : <div className="row-actions">
          <button className="primary-action" disabled={state !== 'idle'} onClick={() => void apply()}>{state === 'applying' ? 'Aplicando…' : 'Aplicar'}</button>
          {!previews && <button className="ghost-button" disabled={state !== 'idle'} onClick={() => void preview()}>{state === 'loading' ? 'Montando a prévia…' : 'Ver prévia'}</button>}
          <button className="ghost-button" disabled={state !== 'idle'} onClick={() => void cancel()}>Cancelar</button>
        </div>}
    {state === 'done' && arquivo && onOpen && <p className="message-created"><button type="button" className="file-chip link-chip" title={arquivo} onClick={() => onOpen(arquivo)}><FileIcon size={14} />Abrir “{arquivo.slice(arquivo.lastIndexOf('/') + 1).replace(/\.pdf$/i, '')}” na Athena</button></p>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

import { useCallback, useEffect, useState } from 'react';
import { native, errorText, savePreferences, type Fact, type MemoryList, type Privacy, type Snapshot } from '../services/Desktop';
import ModelSetup from './ModelSetup';

const privacyLabels: Record<Privacy, string> = { uncertain: 'Não verificada · fica local', sensitive: 'Sensível · sempre local', public: 'Pública · pode ir para a nuvem' };
const originLabels: Record<string, string> = { auto: 'aprendida em conversa', explicit: 'você pediu', manual: 'adicionada por você' };

type Props = { snapshot: Snapshot; refresh: () => Promise<void> };
/** "O que a Ayla lembra": ver, adicionar, editar, classificar e apagar memórias locais. */
export default function MemoryPanel({ snapshot, refresh }: Props) {
  const [data, setData] = useState<MemoryList | null>(null); const [error, setError] = useState('');
  const [draft, setDraft] = useState(''); const [filter, setFilter] = useState('');
  const [editing, setEditing] = useState<{ id: string; content: string } | null>(null); const [busy, setBusy] = useState(false);
  const load = useCallback(async () => { try { setData(await native<MemoryList>('memory_list')); } catch (e) { setError(errorText(e)); } }, []);
  useEffect(() => { void load(); }, [load, snapshot]);
  async function act(work: () => Promise<unknown>) {
    setBusy(true); setError('');
    try { await work(); await load(); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  const save = (fact: Fact, values: { content?: string; privacy?: Privacy }) => act(() => native('memory_save', { id: fact.id, content: values.content ?? fact.content, privacy: values.privacy ?? fact.privacy }));
  const entry = snapshot.catalog.find(e => e.capabilities.includes('embedding'));
  const shown = (data?.facts || []).filter(f => !filter.trim() || f.content.toLowerCase().includes(filter.trim().toLowerCase()));
  return <section className="panel" aria-labelledby="memory-title">
    <h2 id="memory-title">O que a Ayla lembra</h2>
    <p>A Ayla aprende com as conversas e guarda tudo só neste computador. Você pode pedir “lembre que…” ou “esqueça…”, ou ajustar aqui.</p>
    {error && <p role="alert" className="app-error">{error}</p>}
    {data && !data.ready && entry && <div className="memory-activate"><p>Para lembrar e encontrar memórias, a Ayla precisa de um pequeno modelo de memória.</p><ModelSetup entry={entry} refresh={async () => { await refresh(); await load(); }} action="Ativar memória" /></div>}
    <label className="memory-toggle"><input type="checkbox" checked={snapshot.preferences.memory_auto} disabled={busy} onChange={e => { const value = e.target.checked; void act(async () => { await savePreferences({ ...snapshot.preferences, memory_auto: value }); await refresh(); }); }} />Lembrar automaticamente das conversas</label>
    {!snapshot.preferences.memory_auto && <p>Memória automática pausada. As memórias atuais continuam guardadas e usadas nas respostas.</p>}
    <form className="memory-add" onSubmit={e => { e.preventDefault(); if (draft.trim()) void act(async () => { await native('memory_save', { id: null, content: draft, privacy: 'uncertain' }); setDraft(''); }); }}>
      <input aria-label="Nova memória" value={draft} maxLength={1000} disabled={busy} onChange={e => setDraft(e.target.value)} placeholder="Ex.: Prefiro exemplos práticos" />
      <button disabled={busy || draft.trim().length < 3}>Adicionar</button>
    </form>
    {(data?.facts.length || 0) > 5 && <input aria-label="Filtrar memórias" value={filter} onChange={e => setFilter(e.target.value)} placeholder="Filtrar memórias" />}
    {data && !data.facts.length && <p>Nenhuma memória ainda.</p>}
    <ul className="memory-list">{shown.map(fact => <li key={fact.id}>
      {editing?.id === fact.id ? <form onSubmit={e => { e.preventDefault(); void save(fact, { content: editing.content }).then(() => setEditing(null)); }}>
        <textarea aria-label="Editar memória" rows={2} maxLength={1000} value={editing.content} onChange={e => setEditing({ ...editing, content: e.target.value })} />
        <div className="button-row"><button className="primary-action" disabled={busy || editing.content.trim().length < 3}>Salvar</button><button type="button" onClick={() => setEditing(null)}>Cancelar</button></div>
      </form> : <p>{fact.content}</p>}
      <small>{originLabels[fact.origin] || fact.origin} · {new Date(fact.updated_at * 1000).toLocaleDateString('pt-BR')}</small>
      <div className="memory-actions">
        <select aria-label="Privacidade da memória" value={fact.privacy} disabled={busy} onChange={e => void save(fact, { privacy: e.target.value as Privacy })}>{Object.entries(privacyLabels).map(([key, label]) => <option key={key} value={key}>{label}</option>)}</select>
        {editing?.id !== fact.id && <button disabled={busy} onClick={() => setEditing({ id: fact.id, content: fact.content })}>Editar</button>}
        <button disabled={busy} onClick={() => void act(() => native('memory_delete', { id: fact.id }))}>Apagar</button>
      </div>
    </li>)}</ul>
  </section>;
}

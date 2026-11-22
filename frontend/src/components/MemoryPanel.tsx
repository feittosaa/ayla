import { useCallback, useEffect, useState } from 'react';
import Confirm from './Confirm';
import { native, errorText, savePreferences, type Fact, type MemoryList, type Privacy, type Project, type Snapshot } from '../services/Desktop';
import ModelSetup from './ModelSetup';

const privacyLabels: Record<Privacy, string> = { uncertain: 'Não verificada · fica local', sensitive: 'Sensível · sempre local', public: 'Pública · pode ir para a nuvem' };
const originLabels: Record<string, string> = { auto: 'aprendida em conversa', explicit: 'você pediu', manual: 'adicionada por você' };

type Props = { snapshot: Snapshot; refresh: () => Promise<void>; project?: Project | null };
/** "O que a Ayla lembra": ver, adicionar, editar, classificar e apagar memórias locais.
 * Com `project`, mostra só as memórias daquele projeto; sem, as gerais. */
export default function MemoryPanel({ snapshot, refresh, project = null }: Props) {
  const projectId = project?.id ?? null;
  const [data, setData] = useState<MemoryList | null>(null); const [error, setError] = useState('');
  const [draft, setDraft] = useState(''); const [filter, setFilter] = useState('');
  const [editing, setEditing] = useState<{ id: string; content: string } | null>(null); const [busy, setBusy] = useState(false);
  const load = useCallback(async () => { try { setData(await native<MemoryList>('memory_list', { project: projectId })); } catch (e) { setError(errorText(e)); } }, [projectId]);
  useEffect(() => { void load(); }, [load, snapshot]);
  async function act(work: () => Promise<unknown>) {
    setBusy(true); setError('');
    try { await work(); await load(); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  const save = (fact: Fact, values: { content?: string; privacy?: Privacy }) => act(() => native('memory_save', { id: fact.id, content: values.content ?? fact.content, privacy: values.privacy ?? fact.privacy }));
  const entry = snapshot.catalog.find(e => e.capabilities.includes('embedding'));
  const shown = (data?.facts || []).filter(f => !filter.trim() || f.content.toLowerCase().includes(filter.trim().toLowerCase()));
  return <section id={project ? 'memoria-projeto' : 'memoria'} className="settings-group" aria-labelledby="memory-title">
    <h2 id="memory-title">{project ? 'Memória do projeto' : 'Memória'}</h2>
    {error && <p role="alert" className="app-error">{error}</p>}
    <div className="group-card">
      {project ? <div className="group-row stacked"><span className="row-hint">{data && !data.ready ? 'Ative a memória em Configurações para a Ayla lembrar e encontrar memórias.' : `O que a Ayla aprende nas conversas de ${project.name} fica só aqui. As memórias gerais também valem neste projeto.`}</span></div> : <label className="group-row">
        <span className="row-text"><span className="row-title">Lembrar automaticamente</span><span className="row-hint">{snapshot.preferences.memory_auto ? 'A Ayla guarda neste computador o que for importante nas conversas.' : 'Pausado. As memórias atuais continuam guardadas e usadas nas respostas.'}</span></span>
        <input type="checkbox" role="switch" className="switch" checked={snapshot.preferences.memory_auto} disabled={busy} onChange={e => { const value = e.target.checked; void act(async () => { await savePreferences({ ...snapshot.preferences, memory_auto: value }); await refresh(); }); }} />
      </label>}
      {!project && data && !data.ready && entry && <div className="group-row stacked"><span className="row-hint">Para lembrar e encontrar memórias, a Ayla precisa de um pequeno modelo de memória.</span><ModelSetup entry={entry} refresh={async () => { await refresh(); await load(); }} action="Ativar memória" /></div>}
      <div className="group-row stacked">
        <span className="row-title">{project ? 'O que a Ayla lembra deste projeto' : 'O que a Ayla lembra'}</span>
        <span className="row-hint">Você também pode pedir na conversa: “lembre que…” ou “esqueça…”.</span>
        <form className="memory-add" onSubmit={e => { e.preventDefault(); if (draft.trim()) void act(async () => { await native('memory_save', { id: null, content: draft, privacy: 'uncertain', project: projectId }); setDraft(''); await refresh(); }); }}>
          <input aria-label="Nova memória" value={draft} maxLength={1000} disabled={busy} onChange={e => setDraft(e.target.value)} placeholder="Ex.: Prefiro exemplos práticos" />
          <button disabled={busy || draft.trim().length < 3}>Adicionar</button>
        </form>
        {(data?.facts.length || 0) > 5 && <input aria-label="Filtrar memórias" value={filter} onChange={e => setFilter(e.target.value)} placeholder="Filtrar memórias" />}
        {data && !data.facts.length && <p className="row-hint">Nenhuma memória ainda.</p>}
        <ul className="memory-list">{shown.map(fact => <li key={fact.id}>
          {editing?.id === fact.id ? <form onSubmit={e => { e.preventDefault(); void save(fact, { content: editing.content }).then(() => setEditing(null)); }}>
            <textarea aria-label="Editar memória" rows={2} maxLength={1000} value={editing.content} onChange={e => setEditing({ ...editing, content: e.target.value })} />
            <div className="button-row"><button className="primary-action" disabled={busy || editing.content.trim().length < 3}>Salvar</button><button type="button" onClick={() => setEditing(null)}>Cancelar</button></div>
          </form> : <p>{fact.content}</p>}
          <small>{originLabels[fact.origin] || fact.origin} · {new Date(fact.updated_at * 1000).toLocaleDateString('pt-BR')}</small>
          <div className="memory-actions">
            <select aria-label="Privacidade da memória" value={fact.privacy} disabled={busy} onChange={e => void save(fact, { privacy: e.target.value as Privacy })}>{Object.entries(privacyLabels).map(([key, label]) => <option key={key} value={key}>{label}</option>)}</select>
            {editing?.id !== fact.id && <button disabled={busy} onClick={() => setEditing({ id: fact.id, content: fact.content })}>Editar</button>}
            <Confirm disabled={busy} question="Apagar esta memória? A Ayla deixa de lembrar disso." confirm="Apagar" onConfirm={() => void act(() => native('memory_delete', { id: fact.id }))}>Apagar</Confirm>
          </div>
        </li>)}</ul>
      </div>
    </div>
  </section>;
}

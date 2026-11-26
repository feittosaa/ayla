import { useCallback, useEffect, useState } from 'react';
import { native, errorText } from '../services/Desktop';
import BasesDialog from './BasesDialog';
import Confirm from './Confirm';

export type ProjectSource = { id: string; kind: string; root: string; path: string; name: string };
const labels: Record<string,string> = { folder: 'Pasta da Iris', image: 'Imagem da Iris', vault: 'Cofre da Athena', note: 'Nota da Athena', database: 'Base local', catalog: 'Base do catálogo' };
type Group = 'iris' | 'athena' | 'catalog';

export default function ProjectSources({ project, use, open, removed }: {
  project: string; use: (item: ProjectSource) => void; open: (item: ProjectSource) => void; removed: (id: string) => void;
}) {
  const [links, setLinks] = useState<ProjectSource[]>([]);
  const [group, setGroup] = useState<Group | null>(null);
  const [options, setOptions] = useState<ProjectSource[] | null>(null);
  const [query, setQuery] = useState('');
  const [catalog, setCatalog] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const load = useCallback(() => native<ProjectSource[]>('project_links', { project }).then(setLinks), [project]);
  useEffect(() => { void load().catch(e => setError(errorText(e))); }, [load]);
  useEffect(() => {
    if (!group || catalog) return;
    let active = true;
    setOptions(null);
    void native<ProjectSource[]>('project_source_options', { project, group }).then(result => { if (active) setOptions(result); }, e => {
      if (active) { setError(errorText(e)); setOptions([]); }
    });
    return () => { active = false; };
  }, [group, project, catalog]);
  async function run(action: () => Promise<void>) {
    setBusy(true); setError('');
    try { await action(); await load(); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  const matching = options?.filter(item => (item.name + ' ' + labels[item.kind]).toLowerCase().includes(query.toLowerCase()));
  return <section className="settings-group"><h2>Fontes do projeto</h2>
    <p className="row-hint">Escolha uma fonte para usar na próxima mensagem. A consulta acontece neste computador.</p>
    <div className="row-actions">{(['iris','athena','catalog'] as const).map(g => <button key={g} disabled={busy} aria-pressed={group === g}
      onClick={() => { setGroup(group === g ? null : g); setQuery(''); }}>{g === 'iris' ? 'Conectar da Iris' : g === 'athena' ? 'Conectar da Athena' : 'Bases do catálogo'}</button>)}
      <button disabled={busy} onClick={() => void run(() => native('project_import_database', { project }))}>Importar CSV ou SQLite</button></div>
    {group && <div className="group-card"><div className="group-row stacked">
      <input type="search" aria-label="Filtrar fontes" placeholder="Filtrar fontes" value={query} onChange={e => setQuery(e.target.value)} />
      {group === 'catalog' && <button onClick={() => setCatalog(true)}>Instalar base do catálogo ou GitHub…</button>}
      {!options ? 'Carregando…' : !matching?.length ? 'Nenhuma fonte disponível.' : <div className="project-source-options">
        {matching.map(item => { const linked = links.some(s => s.kind === item.kind && s.root === item.root && s.path === item.path);
          return <div className="group-row" key={JSON.stringify([item.kind,item.root,item.path])}><span className="row-text"><span>{item.name}</span><small>{labels[item.kind]}</small></span>
            <button disabled={busy || linked} onClick={() => void run(() => native('project_link', { project, item }))}>{linked ? 'Conectada' : 'Conectar'}</button></div>;
        })}</div>}
    </div></div>}
    {catalog && <BasesDialog onClose={() => setCatalog(false)} />}
    {links.length > 0 && <div className="group-card">{links.map(item => <div key={item.id} className="group-row stacked">
      <span className="row-text"><strong>{item.name}</strong><small>{labels[item.kind]}</small></span><div className="row-actions">
        <button onClick={() => use(item)}>Usar na conversa</button>
        {['folder','image','vault','note'].includes(item.kind) && <button onClick={() => open(item)}>Abrir na {['folder','image'].includes(item.kind) ? 'Iris' : 'Athena'}</button>}
        <Confirm disabled={busy} question={item.kind === 'database' ? 'Remover a cópia da base deste projeto?' : 'Desconectar esta fonte do projeto?'} confirm="Remover"
          onConfirm={() => void run(async () => { await native('project_unlink', { project, id: item.id }); removed(item.id); })}>Remover</Confirm>
      </div></div>)}</div>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

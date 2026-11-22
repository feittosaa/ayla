import { useCallback, useEffect, useState } from 'react';
import { errorText, native, type Preferences } from '../services/Desktop';

type Root = { id: string; kind: 'ayla' | 'vault' | 'project'; name: string; path: string; available: boolean; obsidian: boolean; git: boolean };

/** Configurações › Athena: versionamento com Git e o que fica onde. Cofres se abrem na própria Athena. */
export default function AthenaSettings({ prefs, saving, save }: { prefs: Preferences; saving: boolean; save: (values: Partial<Preferences>, message?: string) => Promise<void> }) {
  const [roots, setRoots] = useState<Root[]>([]); const [error, setError] = useState('');
  const load = useCallback(() => { void native<{ roots: Root[] }>('athena_tree').then(tree => setRoots(tree.roots)).catch(e => setError(errorText(e))); }, []);
  useEffect(() => { load(); window.addEventListener('athena-vaults', load); return () => window.removeEventListener('athena-vaults', load); }, [load]);
  const vaults = roots.filter(r => r.kind === 'vault');
  return <section id="athena" className="settings-group" aria-labelledby="athena-title"><h2 id="athena-title">Athena</h2>
    <div className="group-card">
      <div className="group-row"><span className="row-text"><span className="row-title">Cofres abertos</span><span className="row-hint">{vaults.length ? vaults.map(v => `${v.name}${v.available ? '' : ' (indisponível)'}`).join(' · ') : 'Nenhum. Abra um cofre direto na Athena, pelo botão "Abrir cofre".'}</span></span></div>
      <label className="group-row">
        <span className="row-text"><span className="row-title">Versionar com Git</span><span className="row-hint">Nos cofres que são repositórios, a Ayla segue o obsidian-git de cada um (intervalo, mensagem, puxar antes de enviar). Com o Obsidian aberto, o plugin cuida disso lá. As credenciais são as do próprio Git.</span></span>
        <input type="checkbox" role="switch" className="switch" checked={prefs.athena_git} disabled={saving} onChange={e => void save({ athena_git: e.target.checked }, e.target.checked ? 'Versionamento ligado.' : 'Versionamento desligado.')} />
      </label>
      <div className="group-row"><span className="row-text"><span className="row-title">Plugins do Obsidian</span><span className="row-hint">Os plugins rodam dentro do Obsidian. Use "Abrir no Obsidian" na nota para Kanban, Dataview, Excalidraw e os outros; na Ayla, essas notas aparecem como Markdown.</span></span></div>
    </div>
    <p className="group-note">As notas da Athena e as de cada projeto ficam nos dados da Ayla; as dos cofres ficam nas pastas deles. Apagar move para a lixeira da pasta (.trash). Notas nunca vão para a nuvem.</p>
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

import { useCallback, useEffect, useState, type FormEvent } from 'react';
import Confirm from './Confirm';
import { RecoveryCode } from './AthenaLock';
import { LockIcon } from './Icons';
import { errorText, native, type Preferences } from '../services/Desktop';

/** Chave de recuperação global das pastas e arquivos protegidos: criar (o código aparece uma única
 * vez) ou ativar neste computador com o código anotado. */
function RecoverySettings() {
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [code, setCode] = useState<string | null>(null);
  const [activating, setActivating] = useState(false);
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const refresh = useCallback(() => { void native<{ configured: boolean }>('athena_recovery_status').then(s => setConfigured(s.configured)).catch(e => setError(errorText(e))); }, []);
  useEffect(refresh, [refresh]);
  async function create() {
    setBusy(true); setError(''); setNotice('');
    try { setCode(await native<string>('athena_recovery_create')); refresh(); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  async function activate(event: FormEvent) {
    event.preventDefault();
    setBusy(true); setError(''); setNotice('');
    try { await native('athena_recovery_activate', { code: typed }); setTyped(''); setActivating(false); setNotice('Chave de recuperação ativada neste computador.'); refresh(); }
    catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  return <>
    <div className="group-row">
      <span className="row-text"><span className="row-title">Chave de recuperação</span>
        <span className="row-hint">{configured === null ? 'Verificando…' : configured
          ? 'Ativa neste computador. Ela abre qualquer pasta ou arquivo protegido (Notas e Biblioteca) quando a senha é esquecida; o código fica só com você.'
          : 'Ainda não existe neste computador. Ela é criada ao proteger o primeiro item; se já existe em outro computador, ative-a com o código.'}</span></span>
      <span className="row-actions">
        <button type="button" className="ghost-button" disabled={busy} onClick={() => { setActivating(v => !v); setError(''); }}>Ativar com o código</button>
        {configured
          ? <Confirm className="ghost-button" disabled={busy} question="Criar uma chave nova? A atual continua abrindo cada item até ele ser aberto de novo com a senha, quando passa para a nova." confirm="Criar" onConfirm={() => void create()}>Criar nova</Confirm>
          : <button type="button" className="ghost-button" disabled={busy || configured === null} onClick={() => void create()}>Criar agora</button>}
      </span>
    </div>
    {activating && <form className="group-row recovery-activate" onSubmit={e => void activate(e)}>
      <label className="row-text"><span className="row-title">Código da chave</span>
        <input className="lock-code-input" autoFocus autoComplete="off" spellCheck={false} placeholder="ATHENA-XXXX-XXXX-…" value={typed} onChange={e => setTyped(e.target.value)} /></label>
      <button type="submit" className="primary-action" disabled={busy || !typed.trim()}>Ativar</button>
    </form>}
    {(notice || error) && <p className={error ? 'group-row lock-error' : 'group-row'} role={error ? 'alert' : 'status'}>{error || notice}</p>}
    {code && <div className="dialog-backdrop">
      <div className="dialog lock-dialog" role="dialog" aria-modal="true" aria-labelledby="recovery-title">
        <h2 id="recovery-title"><LockIcon size={18} /> Chave de recuperação</h2>
        <RecoveryCode code={code} onDone={() => { setCode(null); setNotice('Chave nova criada. Os itens passam para ela quando forem abertos com a senha.'); }} />
      </div>
    </div>}
  </>;
}

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
      <RecoverySettings />
      <div className="group-row"><span className="row-text"><span className="row-title">Plugins do Obsidian</span><span className="row-hint">Os plugins rodam dentro do Obsidian. Use "Abrir no Obsidian" na nota para Kanban, Dataview, Excalidraw e os outros; na Ayla, essas notas aparecem como Markdown.</span></span></div>
    </div>
    <p className="group-note">As notas da Athena e as de cada projeto ficam nos dados da Ayla; as dos cofres ficam nas pastas deles. Apagar move para a lixeira da pasta (.trash). Notas nunca vão para a nuvem.</p>
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

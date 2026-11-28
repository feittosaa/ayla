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

type Feriado = { data: string; nome: string; tipo: 'nacional' | 'estadual' | 'municipal' | 'facultativo' };
type FeriadosEstado = { ufs: { sigla: string; nome: string }[]; pesquisa: { buscado_em: number; estaduais: Feriado[]; municipais: Feriado[] } | null; estaduais_embutidos: Feriado[]; nuvem: string | null; buscando: boolean; erro: string | null };
const shortDate = (iso: string) => { const [, m, d] = iso.split('-'); return `${d}/${m}`; };

/** Athena › Calendários: feriados e onde a pessoa mora. Os da cidade são pesquisados uma vez por
 * ano pela nuvem (só o nome da cidade e do estado saem) e ficam guardados aqui. */
function CalendarSettings({ prefs, saving, save }: { prefs: Preferences; saving: boolean; save: (values: Partial<Preferences>, message?: string) => Promise<void> }) {
  const calendar = prefs.calendar ?? { feriados: true, uf: '', cidade: '' };
  const year = new Date().getFullYear();
  const [status, setStatus] = useState<FeriadosEstado | null>(null);
  const [city, setCity] = useState(calendar.cidade);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const load = useCallback(() => { void native<FeriadosEstado>('feriados_estado', { ano: year }).then(setStatus).catch(e => setError(errorText(e))); }, [year]);
  useEffect(() => { load(); }, [load, calendar.uf, calendar.cidade]);
  useEffect(() => { setCity(calendar.cidade); }, [calendar.cidade]);
  useEffect(() => { if (!status?.buscando) return; const t = setInterval(load, 2500); return () => clearInterval(t); }, [status?.buscando, load]);
  const change = (values: Partial<NonNullable<Preferences['calendar']>>, message?: string) => save({ calendar: { ...calendar, ...values } }, message);
  async function search() {
    setBusy(true); setError('');
    // O erro da pesquisa fica guardado e aparece uma vez, na linha dos feriados.
    try { await native('feriados_buscar', { ano: year }); } catch { /* mostrado pelo estado */ } finally { setBusy(false); load(); }
  }
  const pesquisa = status?.pesquisa;
  const estaduais = pesquisa?.estaduais.length ? pesquisa.estaduais : status?.estaduais_embutidos ?? [];
  const place = calendar.cidade && calendar.uf ? `${calendar.cidade} (${calendar.uf})` : '';
  return <>
    <h2 className="subgroup-title" id="athena-calendarios">Calendários</h2>
    <div className="group-card">
      <label className="group-row">
        <span className="row-text"><span className="row-title">Feriados</span><span className="row-hint">Nacionais, do seu estado e da sua cidade, e os pontos facultativos (marcados como tal) no calendário da Athena.</span></span>
        <input type="checkbox" role="switch" className="switch" checked={calendar.feriados} disabled={saving} onChange={e => void change({ feriados: e.target.checked }, e.target.checked ? 'Feriados ligados.' : 'Feriados desligados.')} />
      </label>
      <div className="group-row">
        <span className="row-text"><span className="row-title">Onde você mora</span><span className="row-hint">Para os feriados estaduais e municipais. Fica neste computador; para pesquisar os da cidade, só o nome dela e do estado vão para a nuvem.</span></span>
        <span className="row-actions calendar-place">
          <select aria-label="Estado" value={calendar.uf} disabled={saving || !calendar.feriados} onChange={e => void change({ uf: e.target.value }, 'Estado salvo.')}>
            <option value="">Estado…</option>{status?.ufs.map(u => <option key={u.sigla} value={u.sigla}>{u.nome}</option>)}
          </select>
          <input aria-label="Cidade" placeholder="Cidade" value={city} maxLength={80} disabled={saving || !calendar.feriados || !calendar.uf}
            onChange={e => setCity(e.target.value)} onBlur={() => { if (city.trim() !== calendar.cidade) void change({ cidade: city.trim() }, 'Cidade salva.'); }}
            onKeyDown={e => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); }} />
        </span>
      </div>
      {calendar.feriados && calendar.uf && <div className="group-row calendar-holidays">
        <span className="row-text">
          <span className="row-title">Feriados de {year}{place ? ` em ${place}` : ''}</span>
          <span className="row-hint">{status?.buscando ? 'Pesquisando os feriados da cidade…'
            : !calendar.cidade ? 'Escreva a cidade para incluir os feriados municipais.'
            : pesquisa ? `Pesquisados em ${new Date(pesquisa.buscado_em * 1000).toLocaleDateString('pt-BR')}. A Ayla pesquisa de novo a cada ano.`
            : status?.nuvem ?? (status?.erro ? 'A última pesquisa não deu certo:' : 'A Ayla pesquisa os feriados da cidade quando o calendário abrir.')}</span>
          {status?.erro && !status.buscando && <span className="row-hint lock-error">{status.erro}</span>}
          {estaduais.length > 0 && <span className="row-hint"><b>Estaduais:</b> {estaduais.map(f => `${shortDate(f.data)} ${f.nome}`).join(' · ')}</span>}
          {pesquisa && <span className="row-hint"><b>Municipais:</b> {pesquisa.municipais.length ? pesquisa.municipais.map(f => `${shortDate(f.data)} ${f.nome}`).join(' · ') : 'nenhum encontrado'}</span>}
        </span>
        {calendar.cidade && <span className="row-actions"><button type="button" className="ghost-button" disabled={busy || Boolean(status?.buscando) || Boolean(status?.nuvem)} onClick={() => void search()}>{busy ? 'Buscando…' : pesquisa ? 'Buscar de novo' : 'Buscar agora'}</button></span>}
      </div>}
      {error && <p role="alert" className="group-row lock-error">{error}</p>}
    </div>
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
    <CalendarSettings prefs={prefs} saving={saving} save={save} />
    <p className="group-note">As notas da Athena e as de cada projeto ficam nos dados da Ayla; as dos cofres ficam nas pastas deles. Apagar move para a lixeira da pasta (.trash). Notas nunca vão para a nuvem.</p>
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

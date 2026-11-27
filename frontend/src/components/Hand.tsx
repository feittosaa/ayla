import { useCallback, useEffect, useMemo, useState } from 'react';
import BrandMark from './BrandMark';
import Confirm from './Confirm';
import { FileIcon, PlusIcon, RefreshIcon, ScreenIcon, SidebarIcon, TrashIcon } from './Icons';
import { errorText, native } from '../services/Desktop';
import './Hand.css';
import { nameOf, short, size, when, type Achado, type HandStep, type Lote, type Resultado, type Saida } from '../services/Hand';

type Programa = { id: string; name: string; comment: string; icon: string };
type Estado = { pastas: string[]; padrao: boolean; capturas: string; home: string; ferramentas: Record<string, boolean> };
type Section = 'capturas' | 'buscar' | 'organizar' | 'programas' | 'comandos' | 'historico' | 'pastas';

const sections: { id: Section; label: string }[] = [
  { id: 'capturas', label: 'Capturas de tela' }, { id: 'buscar', label: 'Buscar arquivos' }, { id: 'organizar', label: 'Organizar pastas' },
  { id: 'programas', label: 'Programas' }, { id: 'comandos', label: 'Comandos' }, { id: 'historico', label: 'Histórico' }, { id: 'pastas', label: 'Pastas liberadas' },
];
const tipos: [string, string][] = [['', 'Qualquer tipo'], ['pasta', 'Pastas'], ['imagem', 'Imagens'], ['pdf', 'PDFs'], ['documento', 'Documentos'], ['video', 'Vídeos'], ['audio', 'Áudio'], ['compactado', 'Compactados'], ['instalador', 'Instaladores']];


/** Uma linha do plano, em português: "Mover foto.png → Imagens/foto.png" (relativo à pasta arrumada). */
export function StepLine({ step, home, base }: { step: HandStep; home: string; base?: string }) {
  const show = (path: string) => base && path.startsWith(`${base}/`) ? path.slice(base.length + 1) : short(path, home);
  if (step.acao === 'pasta') return <><span className="hand-verb">Criar pasta</span><span>{show(step.caminho)}</span></>;
  if (step.acao === 'lixeira') return <><span className="hand-verb danger">Lixeira</span><span>{show(step.caminho)}</span></>;
  const sameDir = step.de.slice(0, step.de.lastIndexOf('/')) === step.para.slice(0, step.para.lastIndexOf('/'));
  return <><span className="hand-verb">{sameDir ? 'Renomear' : 'Mover'}</span><span>{show(step.de)} → {sameDir ? nameOf(step.para) : show(step.para)}</span></>;
}

/** Pasta comum dos passos (a que está sendo arrumada), para mostrar caminhos curtos. */
function baseOf(plan: HandStep[]) {
  const first = plan.find(s => s.acao === 'mover');
  if (!first || first.acao !== 'mover') return undefined;
  const dir = first.de.slice(0, first.de.lastIndexOf('/'));
  return plan.every(s => (s.acao === 'mover' ? [s.de, s.para] : [s.caminho]).every(p => p.startsWith(`${dir}/`))) ? dir : undefined;
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
    {state === 'done' && result ? <p role="status" className="hand-done">Feito: {result.lote.feitos.filter(s => s.acao !== 'pasta').length} de {moves}. Dá para desfazer em Histórico.{result.falhas.length ? ` ${result.falhas.length} não ${result.falhas.length === 1 ? 'deu' : 'deram'} certo: ${result.falhas.slice(0, 3).map(([s, why]) => `${s.acao === 'mover' ? nameOf(s.de) : nameOf(s.caminho)} (${why})`).join('; ')}.` : ''}</p>
      : state === 'cancelled' ? <p role="status">Cancelado; nada mudou.</p>
      : <div className="row-actions">
          <button className="primary-action" disabled={state !== 'idle'} onClick={() => void apply()}>{state === 'applying' ? 'Aplicando…' : 'Aplicar'}</button>
          <button className="ghost-button" disabled={state !== 'idle'} onClick={() => { setState('cancelled'); if (cancelKey) localStorage.setItem(cancelKey, '1'); }}>Cancelar</button>
        </div>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

/** Hand: a mão da Ayla no computador. Capturas, busca, organização com prévia, programas e
 * comandos — tudo neste computador, nada vai para a nuvem. */
export default function Hand({ active }: { active: boolean }) {
  const [section, setSection] = useState<Section>(() => (localStorage.getItem('ayla-hand-secao') as Section | null) ?? 'capturas');
  useEffect(() => { localStorage.setItem('ayla-hand-secao', section); }, [section]);
  const [sidebar, setSidebar] = useState(true);
  const [estado, setEstado] = useState<Estado | null>(null);
  const [error, setError] = useState('');
  const load = useCallback(() => { void native<Estado>('hand_estado').then(setEstado).catch(e => setError(errorText(e))); }, []);
  useEffect(() => { if (active) load(); }, [active, load]);
  const home = estado?.home ?? '';
  const fail = useCallback((e: unknown) => setError(errorText(e)), []);
  const open = (path: string, mostrar = false) => void native('hand_abrir', { caminho: path, mostrar }).catch(fail);

  return <div className="chat-layout hand">
    {sidebar && <aside className="sidebar organ-sidebar" aria-label="Seções do Hand">
      <div className="brand"><BrandMark name="hand" size={22} /><span className="brand-name">Hand</span><button className="icon-button organ-hide" data-tip="Esconder a barra lateral" aria-label="Esconder a barra lateral" onClick={() => setSidebar(false)}><SidebarIcon size={18} /></button></div>
      <nav className="hand-sections" aria-label="Seções">
        {sections.map(s => <button key={s.id} className={section === s.id ? 'iris-source active' : 'iris-source'} aria-pressed={section === s.id} onClick={() => setSection(s.id)}><span>{s.label}</span></button>)}
      </nav>
      <p className="panel-muted hand-privacy">Tudo acontece neste computador; nada do Hand vai para a nuvem.</p>
    </aside>}
    <section className="hand-main">
      <header className="note-tabs organ-bar">
        {!sidebar && <button className="icon-button" data-tip="Mostrar a barra lateral" aria-label="Mostrar a barra lateral" onClick={() => setSidebar(true)}><SidebarIcon size={18} /></button>}
        <h2 className="hand-title">{sections.find(s => s.id === section)?.label}</h2>
      </header>
      {error && <p role="alert" className="app-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
      <main className="hand-content">
        {section === 'capturas' && <Captures estado={estado} open={open} fail={fail} />}
        {section === 'buscar' && <Search home={home} open={open} fail={fail} estado={estado} />}
        {section === 'organizar' && <Organize estado={estado} fail={fail} />}
        {section === 'programas' && <Programs active={active} fail={fail} />}
        {section === 'comandos' && <Commands />}
        {section === 'historico' && <History active={active && section === 'historico'} home={home} fail={fail} />}
        {section === 'pastas' && <Folders estado={estado} reload={load} fail={fail} />}
      </main>
    </section>
  </div>;
}

function Captures({ estado, open, fail }: { estado: Estado | null; open: (path: string, reveal?: boolean) => void; fail: (e: unknown) => void }) {
  const [busy, setBusy] = useState(false);
  const [delay, setDelay] = useState(0);
  const [taken, setTaken] = useState<string[]>([]);
  const missing = estado && (!estado.ferramentas.grim || !estado.ferramentas.slurp);
  async function take(tipo: 'tela' | 'regiao' | 'janela') {
    setBusy(true);
    try { const path = await native<string | null>('hand_capturar', { tipo, espera: delay }); if (path) setTaken(prev => [path, ...prev].slice(0, 20)); }
    catch (e) { fail(e); } finally { setBusy(false); }
  }
  return <div className="hand-panel">
    <p className="panel-muted">Salvas em {estado ? estado.capturas.replace(estado.home, '~') : '…'}.</p>
    {missing && <p className="app-error">Para capturar, o sistema precisa do grim e do slurp.</p>}
    <div className="row-actions">
      <button className="primary-action" disabled={busy} onClick={() => void take('tela')}><ScreenIcon size={16} />Tela inteira</button>
      <button disabled={busy} onClick={() => void take('regiao')}>Escolher região</button>
      <button disabled={busy} onClick={() => void take('janela')}>Escolher janela</button>
      <label className="hand-inline">Esperar <select value={delay} onChange={e => setDelay(Number(e.target.value))}>{[0, 3, 5, 10].map(n => <option key={n} value={n}>{n ? `${n} s` : 'nada'}</option>)}</select></label>
    </div>
    {busy && <p role="status" className="panel-muted">{delay ? `Capturando em ${delay} s…` : 'Capturando…'}</p>}
    {taken.length > 0 && <ul className="hand-list">{taken.map(path => <li key={path}>
      <span className="hand-name"><FileIcon size={14} />{nameOf(path)}</span>
      <span className="row-actions"><button className="ghost-button" onClick={() => open(path)}>Abrir</button><button className="ghost-button" onClick={() => open(path, true)}>Mostrar na pasta</button></span>
    </li>)}</ul>}
  </div>;
}

function Search({ home, open, fail, estado }: { home: string; open: (path: string, reveal?: boolean) => void; fail: (e: unknown) => void; estado: Estado | null }) {
  const [query, setQuery] = useState('');
  const [tipo, setTipo] = useState('');
  const [dias, setDias] = useState(0);
  const [found, setFound] = useState<Achado[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [trashed, setTrashed] = useState<string[]>([]);
  async function search(event?: { preventDefault: () => void }) {
    event?.preventDefault();
    setBusy(true);
    try { setFound(await native<Achado[]>('hand_buscar', { busca: query, tipo: tipo || null, dias: dias || null })); setTrashed([]); }
    catch (e) { fail(e); } finally { setBusy(false); }
  }
  const allowed = (path: string) => !!estado?.pastas.some(folder => path.startsWith(`${folder}/`));
  async function toTrash(path: string) {
    try { await native<Resultado>('hand_aplicar', { plano: [{ acao: 'lixeira', caminho: path }], titulo: `Lixeira: ${nameOf(path)}` }); setTrashed(prev => [...prev, path]); }
    catch (e) { fail(e); }
  }
  return <div className="hand-panel">
    <form className="hand-search" onSubmit={e => void search(e)}>
      <input className="organ-search" type="search" autoFocus value={query} placeholder="Nome do arquivo ou pasta (ex.: contrato aluguel)" aria-label="Buscar arquivos" onChange={e => setQuery(e.target.value)} />
      <select aria-label="Tipo" value={tipo} onChange={e => setTipo(e.target.value)}>{tipos.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select>
      <select aria-label="Quando" value={dias} onChange={e => setDias(Number(e.target.value))}>{[[0, 'Qualquer data'], [1, 'Hoje'], [7, 'Última semana'], [30, 'Último mês'], [365, 'Último ano']].map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select>
      <button className="primary-action" disabled={busy || (!query.trim() && !tipo && !dias)}>{busy ? 'Buscando…' : 'Buscar'}</button>
    </form>
    <p className="panel-muted">Procura na pasta pessoal, sem pastas ocultas e sem os dados da Ayla.</p>
    {found && !found.length && <p role="status">Nada encontrado.</p>}
    {found && found.length > 0 && <ul className="hand-list">{found.map(item => <li key={item.path} className={trashed.includes(item.path) ? 'gone' : undefined}>
      <span className="hand-name" title={item.path}>{item.folder ? '📁' : <FileIcon size={14} />}<b>{item.name}</b><small>{short(item.path.slice(0, item.path.lastIndexOf('/')), home)} · {item.folder ? 'pasta' : size(item.bytes)} · {when(item.modified)}</small></span>
      {trashed.includes(item.path) ? <span className="panel-muted">Na lixeira</span> : <span className="row-actions">
        <button className="ghost-button" onClick={() => open(item.path)}>Abrir</button>
        {!item.folder && <button className="ghost-button" onClick={() => open(item.path, true)}>Mostrar na pasta</button>}
        {allowed(item.path) && <Confirm className="icon-button" ariaLabel={`Mandar ${item.name} para a lixeira`} title="Mandar para a lixeira" question={`Mandar “${item.name}” para a lixeira?`} confirm="Lixeira" onConfirm={() => void toTrash(item.path)}><TrashIcon size={15} /></Confirm>}
      </span>}
    </li>)}</ul>}
  </div>;
}

function Organize({ estado, fail }: { estado: Estado | null; fail: (e: unknown) => void }) {
  const [folder, setFolder] = useState('');
  const [mode, setMode] = useState<'tipo' | 'data'>('tipo');
  const [plan, setPlan] = useState<{ steps: HandStep[]; title: string; key: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const chosen = folder || estado?.pastas[0] || '';
  async function prepare() {
    setBusy(true); setPlan(null);
    try {
      const steps = await native<HandStep[]>('hand_planejar', { pasta: chosen, modo: mode });
      setPlan({ steps, title: `${nameOf(chosen)} por ${mode === 'tipo' ? 'tipo' : 'data'}`, key: Date.now() });
    } catch (e) { fail(e); } finally { setBusy(false); }
  }
  return <div className="hand-panel">
    <p className="panel-muted">Arruma os arquivos soltos de uma pasta liberada. Nada muda até você clicar em Aplicar, e dá para desfazer depois.</p>
    <div className="row-actions hand-search">
      <select aria-label="Pasta" value={chosen} onChange={e => { setFolder(e.target.value); setPlan(null); }}>{estado?.pastas.map(p => <option key={p} value={p}>{short(p, estado.home)}</option>)}</select>
      <select aria-label="Como" value={mode} onChange={e => { setMode(e.target.value as 'tipo' | 'data'); setPlan(null); }}><option value="tipo">Por tipo (Imagens, PDFs, Documentos…)</option><option value="data">Por data (ano/mês)</option></select>
      <button className="primary-action" disabled={busy || !chosen} onClick={() => void prepare()}>{busy ? 'Montando…' : 'Ver o plano'}</button>
    </div>
    {plan && <HandPlan key={plan.key} plan={plan.steps} title={plan.title} home={estado?.home ?? ''} />}
  </div>;
}

function Programs({ active, fail }: { active: boolean; fail: (e: unknown) => void }) {
  const [list, setList] = useState<Programa[] | null>(null);
  const [query, setQuery] = useState('');
  const [opened, setOpened] = useState('');
  useEffect(() => { if (active && !list) void native<Programa[]>('hand_programas').then(setList).catch(fail); }, [active, list, fail]);
  const shown = useMemo(() => {
    const plain = (t: string) => t.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
    const q = plain(query.trim());
    return (list ?? []).filter(p => !q || plain(`${p.name} ${p.comment}`).includes(q)).slice(0, 200);
  }, [list, query]);
  return <div className="hand-panel">
    <input className="organ-search" type="search" autoFocus value={query} placeholder="Buscar programa" aria-label="Buscar programa" onChange={e => setQuery(e.target.value)} />
    {opened && <p role="status" className="hand-done">Abrindo {opened}…</p>}
    {!list ? <p role="status" className="panel-muted">Lendo os programas…</p> : <ul className="hand-list">{shown.map(p => <li key={p.id}>
      <span className="hand-name"><b>{p.name}</b>{p.comment && <small>{p.comment}</small>}</span>
      <button className="ghost-button" onClick={() => void native('hand_abrir_programa', { id: p.id }).then(() => setOpened(p.name)).catch(fail)}>Abrir</button>
    </li>)}</ul>}
  </div>;
}

function Commands() {
  const [command, setCommand] = useState('');
  const [running, setRunning] = useState(false);
  const [runs, setRuns] = useState<{ command: string; out?: Saida; error?: string }[]>([]);
  async function run() {
    const text = command.trim();
    setRunning(true);
    try { const out = await native<Saida>('hand_comando', { comando: text }); setRuns(prev => [{ command: text, out }, ...prev].slice(0, 20)); }
    catch (e) { setRuns(prev => [{ command: text, error: errorText(e) }, ...prev].slice(0, 20)); }
    finally { setRunning(false); }
  }
  return <div className="hand-panel">
    <p className="panel-muted">Roda no terminal (bash), na pasta pessoal, por até 60 s. O comando faz o que você faria no terminal: confira antes de confirmar.</p>
    <div className="hand-search">
      <input className="hand-command" value={command} spellCheck={false} placeholder="ex.: df -h" aria-label="Comando" onChange={e => setCommand(e.target.value)} />
      <Confirm className="primary-action" disabled={running || !command.trim()} question="Rodar este comando no seu computador?" confirm="Rodar" onConfirm={() => void run()}>{running ? 'Rodando…' : 'Rodar'}</Confirm>
    </div>
    {runs.map((r, i) => <section key={i} className="hand-run">
      <code>$ {r.command}</code>
      {r.error ? <p className="app-error">{r.error}</p> : r.out && <>
        {r.out.stdout && <pre>{r.out.stdout}</pre>}
        {r.out.stderr && <pre className="hand-stderr">{r.out.stderr}</pre>}
        <small className="panel-muted">{r.out.tempo_esgotado ? 'Parado: passou de 60 s.' : `Terminou com código ${r.out.code ?? '?'}.`}{r.out.cortado ? ' Saída cortada (64 KB).' : ''}</small>
      </>}
    </section>)}
  </div>;
}

function History({ active, home, fail }: { active: boolean; home: string; fail: (e: unknown) => void }) {
  const [lotes, setLotes] = useState<Lote[] | null>(null);
  const [message, setMessage] = useState('');
  const load = useCallback(() => { void native<Lote[]>('hand_historico').then(setLotes).catch(fail); }, [fail]);
  useEffect(() => { if (active) load(); }, [active, load]);
  async function undo(lote: Lote) {
    try {
      const r = await native<{ voltaram: number; problemas: string[] }>('hand_desfazer', { id: lote.id });
      setMessage(`Desfeito: ${r.voltaram} ${r.voltaram === 1 ? 'arquivo voltou' : 'arquivos voltaram'}.${r.problemas.length ? ` ${r.problemas.slice(0, 3).join('; ')}.` : ''}`);
      load();
    } catch (e) { fail(e); }
  }
  return <div className="hand-panel">
    <div className="row-actions"><button className="icon-button" aria-label="Atualizar" data-tip="Atualizar" onClick={load}><RefreshIcon size={16} /></button></div>
    {message && <p role="status" className="hand-done">{message}</p>}
    {lotes && !lotes.length && <p className="panel-muted">Nada feito ainda. O que o Hand mudar nos seus arquivos aparece aqui, para desfazer.</p>}
    {lotes?.map(lote => <details key={lote.id} className="hand-batch">
      <summary><b>{lote.titulo || 'Mudanças'}</b><small>{when(lote.quando)} · {lote.feitos.filter(s => s.acao !== 'pasta').length} itens{lote.desfeito ? ' · desfeito' : ''}</small>
        {!lote.desfeito && <Confirm className="ghost-button" question="Desfazer estas mudanças?" confirm="Desfazer" onConfirm={() => void undo(lote)}>Desfazer</Confirm>}</summary>
      <ul className="hand-steps">{lote.feitos.slice(0, 200).map((step, i) => <li key={i}><StepLine step={step} home={home} base={baseOf(lote.feitos)} /></li>)}</ul>
    </details>)}
  </div>;
}

function Folders({ estado, reload, fail }: { estado: Estado | null; reload: () => void; fail: (e: unknown) => void }) {
  const save = (pastas: string[] | null) => void native('hand_pastas', { pastas }).then(reload).catch(fail);
  async function add() {
    try { const folder = await native<string | null>('athena_pick_folder'); if (folder && estado) save([...estado.pastas, folder]); }
    catch (e) { fail(e); }
  }
  return <div className="hand-panel">
    <p className="panel-muted">O Hand procura na pasta pessoal inteira, mas só move, renomeia, cria pastas ou manda para a lixeira dentro destas pastas (e das que estão dentro delas).</p>
    <ul className="hand-list">{estado?.pastas.map(folder => <li key={folder}>
      <span className="hand-name">📁 <b>{short(folder, estado.home)}</b></span>
      <Confirm className="ghost-button" question={`Tirar ${short(folder, estado.home)} das pastas liberadas?`} confirm="Tirar" onConfirm={() => save(estado.pastas.filter(p => p !== folder))}>Tirar</Confirm>
    </li>)}</ul>
    <div className="row-actions">
      <button onClick={() => void add()}><PlusIcon size={15} />Liberar pasta</button>
      {estado && !estado.padrao && <button className="ghost-button" onClick={() => save(null)}>Voltar às padrão</button>}
    </div>
  </div>;
}

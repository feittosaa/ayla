import { useCallback, useEffect, useState } from 'react';
import BrandMark from './BrandMark';
import ChatHand, { type HandCard } from './ChatHand';
import Confirm from './Confirm';
import HandDotfiles from './HandDotfiles';
import HandPrograms from './HandPrograms';
import HandRoutines from './HandRoutines';
import HandShortcuts from './HandShortcuts';
import { StepLine } from './HandPlan';
import { FileIcon, FolderIcon, PlusIcon, RefreshIcon, SidebarIcon, TrashIcon } from './Icons';
import { errorText, native } from '../services/Desktop';
import './Hand.css';
import { baseOf, nameOf, short, size, when, type Achado, type Lote, type Resultado, type Saida } from '../services/Hand';

type Estado = { pastas: string[]; padrao: boolean; capturas: string; home: string; ferramentas: Record<string, boolean> };
type Section = 'tarefa' | 'rotinas' | 'buscar' | 'programas' | 'atalhos' | 'dotfiles' | 'comandos' | 'historico' | 'pastas';

/** Seções: "Pedir uma tarefa" fica no alto, como a nova conversa da Ayla; o resto são ferramentas.
 * Capturas, organizar pastas, teclado, mouse e janelas são pedidos na tarefa (ou na conversa). */
const tools: { id: Section; label: string; cmd: string }[] = [
  { id: 'rotinas', label: 'Rotinas', cmd: 'cron' }, { id: 'buscar', label: 'Buscar arquivos', cmd: 'find' }, { id: 'programas', label: 'Programas', cmd: 'apps' },
  { id: 'atalhos', label: 'Atalhos e macros', cmd: 'bind' }, { id: 'dotfiles', label: 'Dotfiles', cmd: 'git' }, { id: 'comandos', label: 'Comandos', cmd: 'sh' },
  { id: 'historico', label: 'Histórico', cmd: 'log' }, { id: 'pastas', label: 'Pastas liberadas', cmd: 'chmod' },
];
const titles: Record<Section, string> = { tarefa: 'Pedir uma tarefa', rotinas: 'Rotinas', buscar: 'Buscar arquivos', programas: 'Programas', atalhos: 'Atalhos e macros', dotfiles: 'Dotfiles', comandos: 'Comandos', historico: 'Histórico', pastas: 'Pastas liberadas' };
const cmds: Record<Section, string> = { tarefa: 'hand', rotinas: 'cron', buscar: 'find', programas: 'apps', atalhos: 'bind', dotfiles: 'git', comandos: 'sh', historico: 'log', pastas: 'chmod' };
const tipos: [string, string][] = [['imagem', 'Imagens'], ['pdf', 'PDFs'], ['documento', 'Textos e documentos'], ['video', 'Vídeos'], ['audio', 'Áudio'], ['compactado', 'Compactados'], ['instalador', 'Instaladores'], ['pasta', 'Pastas']];

/** Hand: a mão da Ayla no computador. Tudo neste computador, nada vai para a nuvem. */
export default function Hand({ active }: { active: boolean }) {
  const [section, setSection] = useState<Section>(() => { const saved = localStorage.getItem('ayla-hand-secao') as Section | null; return saved && saved in titles ? saved : 'tarefa'; });
  useEffect(() => { localStorage.setItem('ayla-hand-secao', section); }, [section]);
  const [sidebar, setSidebar] = useState(true);
  const [estado, setEstado] = useState<Estado | null>(null);
  const [error, setError] = useState('');
  const load = useCallback(() => { void native<Estado>('hand_estado').then(setEstado).catch(e => setError(errorText(e))); }, []);
  useEffect(() => { if (active) load(); }, [active, load]);
  const home = estado?.home ?? '';
  const fail = useCallback((e: unknown) => setError(errorText(e)), []);
  const open = (path: string, mostrar = false) => void native('hand_abrir', { caminho: path, mostrar }).catch(fail);
  const user = home ? home.slice(home.lastIndexOf('/') + 1) : 'você';

  return <div className="chat-layout hand">
    {sidebar && <aside className="sidebar organ-sidebar hand-sidebar" aria-label="Seções do Hand">
      <div className="brand"><BrandMark name="hand" size={22} /><span className="brand-name">Hand</span><button className="icon-button organ-hide" data-tip="Esconder a barra lateral" aria-label="Esconder a barra lateral" onClick={() => setSidebar(false)}><SidebarIcon size={18} /></button></div>
      <button className={section === 'tarefa' ? 'new-chat hand-new active' : 'new-chat hand-new'} aria-pressed={section === 'tarefa'} onClick={() => setSection('tarefa')}><span className="hand-prompt" aria-hidden="true">&gt;_</span>Pedir uma tarefa</button>
      <nav className="conversation-list hand-sections" aria-label="Ferramentas">
        <p className="sidebar-label">Ferramentas</p>
        {tools.map(s => <button key={s.id} className="conversation-item hand-tool" aria-current={section === s.id ? 'page' : undefined} onClick={() => setSection(s.id)}>
          <span>{s.label}</span><small aria-hidden="true">{s.cmd}</small></button>)}
      </nav>
      <p className="panel-muted hand-privacy">Tudo acontece neste computador; nada do Hand vai para a nuvem.</p>
    </aside>}
    <section className="hand-main">
      <header className="note-tabs organ-bar">
        {!sidebar && <button className="icon-button" data-tip="Mostrar a barra lateral" aria-label="Mostrar a barra lateral" onClick={() => setSidebar(true)}><SidebarIcon size={18} /></button>}
        <h2 className="hand-title"><span className="hand-path" aria-hidden="true">{user}@hand:~$ {cmds[section]}</span><span className="hand-caret" aria-hidden="true" /><span className="visually-hidden">{titles[section]}</span></h2>
        <span className="hand-heading">{titles[section]}</span>
      </header>
      {error && <p role="alert" className="app-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
      <main className="hand-content">
        {section === 'tarefa' && <Task />}
        {section === 'rotinas' && <HandRoutines active={active && section === 'rotinas'} fail={fail} />}
        {section === 'buscar' && <Search home={home} open={open} fail={fail} estado={estado} />}
        {section === 'programas' && <HandPrograms active={active && section === 'programas'} fail={fail} />}
        {section === 'atalhos' && <HandShortcuts active={active && section === 'atalhos'} fail={fail} />}
        {section === 'dotfiles' && <HandDotfiles active={active && section === 'dotfiles'} />}
        {section === 'comandos' && <Commands />}
        {section === 'historico' && <History active={active && section === 'historico'} home={home} fail={fail} />}
        {section === 'pastas' && <Folders estado={estado} reload={load} fail={fail} />}
      </main>
    </section>
  </div>;
}

function Search({ home, open, fail, estado }: { home: string; open: (path: string, reveal?: boolean) => void; fail: (e: unknown) => void; estado: Estado | null }) {
  const [query, setQuery] = useState('');
  const [kinds, setKinds] = useState<string[]>([]);
  const [dias, setDias] = useState(0);
  const [ocultos, setOcultos] = useState(false);
  const [tudo, setTudo] = useState(false);
  const [found, setFound] = useState<Achado[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [trashed, setTrashed] = useState<string[]>([]);
  const toggle = (kind: string) => setKinds(prev => prev.includes(kind) ? prev.filter(k => k !== kind) : [...prev, kind]);
  async function search(event?: { preventDefault: () => void }) {
    event?.preventDefault();
    setBusy(true);
    try { setFound(await native<Achado[]>('hand_buscar', { busca: query, tipos: kinds, dias: dias || null, ocultos, tudo })); setTrashed([]); }
    catch (e) { fail(e); } finally { setBusy(false); }
  }
  const allowed = (path: string) => !!estado?.pastas.some(folder => path.startsWith(`${folder}/`)) && !path.slice(home.length).includes('/.');
  async function toTrash(path: string) {
    try { await native<Resultado>('hand_aplicar', { plano: [{ acao: 'lixeira', caminho: path }], titulo: `Lixeira: ${nameOf(path)}` }); setTrashed(prev => [...prev, path]); }
    catch (e) { fail(e); }
  }
  const where = `${tudo ? 'Todo o computador (menos /proc, /sys, /dev e /run)' : 'Pasta pessoal'}${ocultos ? ', com ocultos' : ', sem ocultos'}; os dados da Ayla ficam de fora.`;
  return <div className="hand-panel">
    <form className="hand-search" onSubmit={e => void search(e)}>
      <input className="organ-search" type="search" autoFocus value={query} placeholder="Nome do arquivo ou pasta (ex.: contrato aluguel)" aria-label="Buscar arquivos" onChange={e => setQuery(e.target.value)} />
      <select aria-label="Quando" value={dias} onChange={e => setDias(Number(e.target.value))}>{[[0, 'Qualquer data'], [1, 'Hoje'], [7, 'Última semana'], [30, 'Último mês'], [365, 'Último ano']].map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select>
      <button className="primary-action" disabled={busy || (!query.trim() && !kinds.length && !dias)}>{busy ? 'Buscando…' : 'Buscar'}</button>
    </form>
    <fieldset className="hand-options">
      <legend className="visually-hidden">Opções da busca</legend>
      <label className="hand-check"><input type="checkbox" checked={ocultos} onChange={e => setOcultos(e.target.checked)} />Mostrar arquivos ocultos</label>
      <label className="hand-check"><input type="checkbox" checked={tudo} onChange={e => setTudo(e.target.checked)} />Procurar em todo o computador</label>
    </fieldset>
    <fieldset className="hand-options" aria-label="Tipos">
      <legend className="hand-options-label">Tipo</legend>
      {tipos.map(([v, l]) => <label key={v} className={kinds.includes(v) ? 'hand-chip on' : 'hand-chip'}><input type="checkbox" checked={kinds.includes(v)} onChange={() => toggle(v)} />{l}</label>)}
      {kinds.length > 0 && <button type="button" className="link-button" onClick={() => setKinds([])}>Qualquer tipo</button>}
    </fieldset>
    <p className="panel-muted">{where}</p>
    {found && !found.length && <p role="status">Nada encontrado.</p>}
    {found && found.length > 0 && <ul className="hand-list">{found.map(item => <li key={item.path} className={trashed.includes(item.path) ? 'gone' : undefined}>
      <span className="hand-name" title={item.path}>{item.folder ? <span className="hand-folder-icon"><FolderIcon size={15} /></span> : <FileIcon size={14} />}<b>{item.name}</b><small>{short(item.path.slice(0, item.path.lastIndexOf('/')), home)} · {item.folder ? 'pasta' : size(item.bytes)} · {when(item.modified)}</small></span>
      {trashed.includes(item.path) ? <span className="panel-muted">Na lixeira</span> : <span className="row-actions">
        <button className="ghost-button" onClick={() => open(item.path)}>Abrir</button>
        {!item.folder && <button className="ghost-button" onClick={() => open(item.path, true)}>Mostrar na pasta</button>}
        {allowed(item.path) && <Confirm className="icon-button" ariaLabel={`Mandar ${item.name} para a lixeira`} title="Mandar para a lixeira" question={`Mandar “${item.name}” para a lixeira?`} confirm="Lixeira" onConfirm={() => void toTrash(item.path)}><TrashIcon size={15} /></Confirm>}
      </span>}
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
      <span className="hand-name"><span className="hand-folder-icon"><FolderIcon size={16} /></span><b>{short(folder, estado.home)}</b></span>
      <Confirm className="ghost-button" question={`Tirar ${short(folder, estado.home)} das pastas liberadas?`} confirm="Tirar" onConfirm={() => save(estado.pastas.filter(p => p !== folder))}>Tirar</Confirm>
    </li>)}</ul>
    <div className="row-actions">
      <button onClick={() => void add()}><PlusIcon size={15} />Liberar pasta</button>
      {estado && !estado.padrao && <button className="ghost-button" onClick={() => save(null)}>Voltar às padrão</button>}
    </div>
  </div>;
}

type Pedido = { key: number; texto: string; card: HandCard | null };

function Task() {
  const [goal, setGoal] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [items, setItems] = useState<Pedido[]>([]);
  async function plan(event: { preventDefault: () => void }) {
    event.preventDefault();
    setBusy(true); setError('');
    try { const result = await native<Omit<Pedido, 'key'>>('hand_tarefa_planejar', { objetivo: goal }); setItems(prev => [{ ...result, key: Date.now() }, ...prev].slice(0, 5)); setGoal(''); }
    catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  return <div className="hand-panel">
    <p className="panel-muted">Diga o que fazer no computador: tirar um print, arrumar uma pasta, mover o que está na Iris, abrir e usar programas, digitar, clicar, mexer em janelas. O que dá para fazer sem a tela vem com prévia; o resto vira um plano para você aprovar, e a Ayla para antes de algo que não dá para desfazer.</p>
    <form className="hand-search" onSubmit={e => void plan(e)}>
      <input className="organ-search" value={goal} autoFocus placeholder="ex.: tira um print da tela · organiza meus Downloads por tipo · fecha o Discord" aria-label="Tarefa" onChange={e => setGoal(e.target.value)} />
      <button className="primary-action" disabled={busy || !goal.trim()}>{busy ? 'Preparando…' : 'Preparar'}</button>
    </form>
    {error && <p role="alert" className="app-error">{error}</p>}
    {items.map(item => <section key={item.key} className="hand-request">
      <p>{item.texto.replace(/\*\*|`/g, '')}</p>
      {item.card && <ChatHand item={item.card} />}
    </section>)}
  </div>;
}

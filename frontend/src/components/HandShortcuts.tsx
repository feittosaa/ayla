import { useCallback, useEffect, useMemo, useState } from 'react';
import Confirm from './Confirm';
import { PlusIcon, TrashIcon } from './Icons';
import { native } from '../services/Desktop';

type Combo = { mods: string[]; tecla: string };
type Grupo = { id: string; combo: Combo; descricao: string };
type Atalho = { id: string; combo: Combo; flags: string; descricao: string; acao: string; arg: string; fonte: 'dotfiles' | 'pessoal' | 'ayla'; arquivo: string; secao: string; oculto: boolean; submapa: string; estado: 'ativo' | 'desativado' | 'trocado'; substitui: Combo | null; grupo: Grupo | null };
type Botao = 'esquerdo' | 'direito' | 'meio';
type Passo =
  | { tipo: 'digitar'; texto: string }
  | { tipo: 'teclas'; atalho: string }
  | { tipo: 'gravado'; rotulo: string; teclas: [number, boolean][] }
  | { tipo: 'clicar'; botao: Botao; x: number; y: number; vezes: number }
  | { tipo: 'arrastar'; botao: Botao; de: [number, number]; para: [number, number] }
  | { tipo: 'rolar'; dentes: number }
  | { tipo: 'esperar'; ms: number }
  | { tipo: 'abrir'; programa: string; nome: string };
type Macro = { id: string; nome: string; passos: Passo[] };
type Programa = { id: string; name: string };

const MODS = ['SUPER', 'CTRL', 'ALT', 'SHIFT'];
const MOD_LABEL: Record<string, string> = { SUPER: 'Super', CTRL: 'Ctrl', ALT: 'Alt', SHIFT: 'Shift', CAPS: 'Caps', MOD2: 'Mod2', MOD3: 'Mod3', MOD5: 'Mod5' };
const KEY_LABEL: Record<string, string> = {
  'mouse:272': 'Clique esquerdo', 'mouse:273': 'Clique direito', 'mouse:274': 'Clique do meio', 'mouse:275': 'Botão lateral', 'mouse:276': 'Botão lateral 2',
  mouse_up: 'Roda ↑', mouse_down: 'Roda ↓', return: 'Enter', prior: 'PgUp', next: 'PgDn', slash: '/', backslash: '\\', period: '.', comma: ',', minus: '-', equal: '=',
  semicolon: ';', apostrophe: "'", grave: '`', bracketleft: '[', bracketright: ']', space: 'Espaço', escape: 'Esc', print: 'Print', up: '↑', down: '↓', left: '←', right: '→',
  super_l: 'Super', super_r: 'Super direito', catchall: 'Qualquer tecla', backspace: '⌫', tab: 'Tab', delete: 'Delete',
};
/** Teclas pelo código do X (code:10 = 1 … code:19 = 0; teclado numérico). */
const CODES: Record<number, string> = { 10: '1', 11: '2', 12: '3', 13: '4', 14: '5', 15: '6', 16: '7', 17: '8', 18: '9', 19: '0', 87: 'Num 1', 88: 'Num 2', 89: 'Num 3', 83: 'Num 4', 84: 'Num 5', 85: 'Num 6', 79: 'Num 7', 80: 'Num 8', 81: 'Num 9', 90: 'Num 0' };
const SUMMARY: Record<string, string> = { hash: '1…0', 'page_↑/↓': 'PgUp/PgDn', 'scroll ↑/↓': 'Roda ↑/↓' };
function keyLabel(k: string) {
  const code = /^code:(\d+)$/.exec(k);
  if (code) return CODES[Number(code[1])] ?? k;
  return SUMMARY[k.toLowerCase()] ?? KEY_LABEL[k.toLowerCase()] ?? (k.startsWith('XF86') ? k.slice(4).replace(/([a-z])([A-Z])/g, '$1 $2') : k.length === 1 ? k.toUpperCase() : k);
}
/** O que um atalho sem descrição faz, em português. */
function describe(a: Pick<Atalho, 'acao' | 'arg' | 'descricao'>) {
  if (a.descricao) return a.descricao;
  const ws = /workspace_action\.sh\s+(\w+)\s+(\S+)/.exec(a.arg);
  const [acao, arg] = ws ? [ws[1], ws[2]] : [a.acao, a.arg.trim()];
  const where = (x: string) => x === 'special' ? 'a área de rascunho' : /^r?\+1$/.test(x) ? 'o próximo workspace' : /^r?-1$/.test(x) ? 'o workspace anterior' : `o workspace ${x}`;
  const dir: Record<string, string> = { l: 'à esquerda', r: 'à direita', u: 'acima', d: 'abaixo' };
  switch (acao) {
    case 'workspace': return `Ir para ${where(arg)}`;
    case 'movetoworkspace': return `Levar a janela para ${where(arg)}`;
    case 'movetoworkspacesilent': return `Mandar a janela para ${where(arg)}`;
    case 'movefocus': return `Focar a janela ${dir[arg] ?? arg}`;
    case 'movewindow': return `Mover a janela ${dir[arg] ?? arg}`;
    case 'killactive': return 'Fechar a janela';
    case 'togglefloating': return 'Janela flutuante';
    case 'fullscreen': return 'Tela cheia';
    case 'togglespecialworkspace': return 'Mostrar a área de rascunho';
    default: return '';
  }
}
const comboLabel = (c: Combo) => [...c.mods.map(m => MOD_LABEL[m] ?? m), keyLabel(c.tecla)].filter(Boolean);
const actionLabel = (a: Pick<Atalho, 'acao' | 'arg'>) => a.acao === 'exec' ? a.arg : `${a.acao}${a.arg ? ` ${a.arg}` : ''}`;
const plain = (t: string) => t.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();

/** Tecla do navegador → nome do Hyprland (XKB). */
function hyprKey(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (/^F\d{1,2}$/.test(code)) return code;
  if (/^Numpad\d$/.test(code)) return `KP_${code.slice(6)}`;
  const map: Record<string, string> = { Enter: 'Return', NumpadEnter: 'KP_Enter', Space: 'space', Tab: 'Tab', Escape: 'Escape', Backspace: 'BackSpace', Delete: 'Delete', Insert: 'Insert',
    Home: 'Home', End: 'End', PageUp: 'Prior', PageDown: 'Next', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right', Slash: 'slash', Backslash: 'backslash',
    Period: 'period', Comma: 'comma', Minus: 'minus', Equal: 'equal', Semicolon: 'semicolon', Quote: 'apostrophe', Backquote: 'grave', BracketLeft: 'bracketleft', BracketRight: 'bracketright', PrintScreen: 'Print' };
  return map[code] ?? null;
}

function Keys({ combo }: { combo: Combo }) {
  return <span className="hand-keys">{comboLabel(combo).map((k, i) => <kbd key={i}>{k}</kbd>)}</span>;
}

/** Escolhe a combinação: marcando os modificadores e a tecla, ou apertando-as (o Hyprland segura
 * as que já são atalhos dele; nesse caso, marque à mão). */
function ComboInput({ value, onChange }: { value: Combo; onChange: (c: Combo) => void }) {
  const [listening, setListening] = useState(false);
  useEffect(() => {
    if (!listening) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault(); e.stopPropagation();
      if (['MetaLeft', 'MetaRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'OSLeft', 'OSRight'].includes(e.code)) return;
      const key = hyprKey(e.code);
      if (!key) return;
      if (e.code === 'Escape' && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) { setListening(false); return; }
      onChange({ mods: MODS.filter(m => (m === 'SUPER' && e.metaKey) || (m === 'CTRL' && e.ctrlKey) || (m === 'ALT' && e.altKey) || (m === 'SHIFT' && e.shiftKey)), tecla: key });
      setListening(false);
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [listening, onChange]);
  return <div className="hand-combo">
    {MODS.map(m => <label key={m} className={value.mods.includes(m) ? 'hand-chip on' : 'hand-chip'}><input type="checkbox" checked={value.mods.includes(m)} onChange={e => onChange({ ...value, mods: MODS.filter(x => x === m ? e.target.checked : value.mods.includes(x)) })} />{MOD_LABEL[m]}</label>)}
    <input className="hand-key-input" value={value.tecla} aria-label="Tecla" placeholder="Tecla" spellCheck={false} onChange={e => onChange({ ...value, tecla: e.target.value.trim() })} />
    <button type="button" className={listening ? 'ghost-button hand-listening' : 'ghost-button'} onClick={() => setListening(v => !v)}>{listening ? 'Aperte a combinação… (Esc cancela)' : 'Gravar combinação'}</button>
  </div>;
}

type Resposta = { ok?: boolean; conflito?: Atalho };

type Entry = Atalho | { grupo: Grupo; members: Atalho[] };
/** Sem "Mostrar internos", os atalhos internos de um mesmo resumo da cola viram uma linha. */
function collapse(items: Atalho[], internos: boolean): Entry[] {
  if (internos) return items;
  const out: Entry[] = [];
  for (const a of items) {
    if (!a.grupo || !a.oculto) { out.push(a); continue; }
    const g = out.find(e => 'members' in e && e.grupo.id === a.grupo!.id);
    if (g && 'members' in g) g.members.push(a); else out.push({ grupo: a.grupo, members: [a] });
  }
  return out;
}


/** Atalhos e macros: tudo o que está no Hyprland e as macros da Ayla. */
export default function HandShortcuts({ active, fail }: { active: boolean; fail: (e: unknown) => void }) {
  const [tab, setTab] = useState<'atalhos' | 'macros'>(() => localStorage.getItem('ayla-hand-atalhos-aba') === 'macros' ? 'macros' : 'atalhos');
  useEffect(() => { localStorage.setItem('ayla-hand-atalhos-aba', tab); }, [tab]);
  const [atalhos, setAtalhos] = useState<Atalho[] | null>(null);
  const [hyprland, setHyprland] = useState(true);
  const [arquivo, setArquivo] = useState('');
  const load = useCallback(() => native<{ atalhos: Atalho[]; hyprland: boolean; arquivo: string }>('hand_atalhos').then(r => { setAtalhos(r.atalhos); setHyprland(r.hyprland); setArquivo(r.arquivo); }).catch(fail), [fail]);
  useEffect(() => { if (active) void load(); }, [active, load]);
  return <div className="hand-panel">
    <div className="hand-tabs" role="tablist" aria-label="Atalhos e macros">
      <button role="tab" aria-selected={tab === 'atalhos'} className={tab === 'atalhos' ? 'hand-tab on' : 'hand-tab'} onClick={() => setTab('atalhos')}>Atalhos</button>
      <button role="tab" aria-selected={tab === 'macros'} className={tab === 'macros' ? 'hand-tab on' : 'hand-tab'} onClick={() => setTab('macros')}>Macros</button>
    </div>
    {!hyprland ? <p className="panel-muted">Por enquanto, a Ayla gerencia os atalhos do Hyprland, e não achei a configuração dele.</p>
      : tab === 'atalhos' ? <Shortcuts list={atalhos} arquivo={arquivo} reload={load} fail={fail} />
      : <Macros atalhos={atalhos ?? []} reloadShortcuts={load} fail={fail} active={active} />}
  </div>;
}

function Shortcuts({ list, arquivo, reload, fail }: { list: Atalho[] | null; arquivo: string; reload: () => Promise<void>; fail: (e: unknown) => void }) {
  const [query, setQuery] = useState('');
  const [fonte, setFonte] = useState('');
  const [internos, setInternos] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [notice, setNotice] = useState('');
  const [open, setOpen] = useState<string[]>([]);
  const shown = useMemo(() => {
    const q = plain(query.trim());
    return (list ?? []).filter(a => (internos || !a.oculto || a.grupo) && !a.submapa
      && (!fonte || (fonte === 'desativado' ? a.estado !== 'ativo' : a.fonte === fonte))
      && (!q || plain(`${comboLabel(a.combo).join(' ')} ${describe(a)} ${actionLabel(a)} ${a.secao} ${a.grupo?.descricao ?? ''}`).includes(q)));
  }, [list, query, fonte, internos]);
  const groups = useMemo(() => {
    const out: [string, Atalho[]][] = [];
    for (const a of shown) {
      const name = a.fonte === 'ayla' ? 'Criados pela Ayla' : a.secao || (a.fonte === 'pessoal' ? 'Seus atalhos' : 'Outros');
      const g = out.find(([n]) => n === name);
      if (g) g[1].push(a); else out.push([name, [a]]);
    }
    return out;
  }, [shown]);
  const row = (a: Atalho) => <li key={a.id} className={a.estado !== 'ativo' ? 'off' : undefined}>
          <Keys combo={a.combo} />
          <span className="hand-bind-what"><b>{describe(a) || actionLabel(a)}</b>{describe(a) && <small>{actionLabel(a)}</small>}
            {a.substitui && <small>no lugar de {comboLabel(a.substitui).join('+')}</small>}
            {a.estado !== 'ativo' && <small className="hand-bind-state">{a.estado === 'trocado' ? 'trocado por outra tecla' : 'desativado'}</small>}</span>
          <span className="row-actions">
            {a.estado === 'ativo' ? <>
              <button className="ghost-button" onClick={() => setEditing(editing === a.id ? null : a.id)}>Trocar tecla</button>
              {a.fonte === 'ayla'
                ? <Confirm className="icon-button" ariaLabel="Apagar atalho" title="Apagar atalho" question="Apagar este atalho da Ayla?" confirm="Apagar" onConfirm={() => void change({ op: 'desativar', id: a.id }, 'Atalho apagado.')}><TrashIcon size={15} /></Confirm>
                : <button className="ghost-button" onClick={() => void change({ op: 'desativar', id: a.id }, `${comboLabel(a.combo).join('+')} desativado. Dá para restaurar.`)}>Desativar</button>}
            </> : <button className="ghost-button" onClick={() => void change({ op: 'restaurar', id: a.id }, 'Atalho restaurado.')}>Restaurar</button>}
          </span>
          {editing === a.id && <ChangeKey atalho={a} change={change} close={() => setEditing(null)} />}
        </li>;
  async function change(op: Record<string, unknown>, ok: string): Promise<Resposta> {
    try {
      const r = await native<Resposta>('hand_atalho_mudar', { op });
      if (r.ok) { setNotice(ok); await reload(); }
      return r;
    } catch (e) { fail(e); return {}; }
  }
  return <>
    <p className="panel-muted">Todos os seus atalhos do Hyprland. O que você muda aqui fica em <code>{arquivo.replace(/^\/home\/[^/]+/, '~')}</code>, carregado por último: vale por cima dos outros. Restaurar volta ao original.</p>
    <div className="hand-search">
      <input className="organ-search" type="search" value={query} placeholder="Buscar atalho (tecla, o que faz, comando)" aria-label="Buscar atalho" onChange={e => setQuery(e.target.value)} />
      <button className="primary-action" onClick={() => setCreating(v => !v)}><PlusIcon size={15} />Novo atalho</button>
    </div>
    <div className="hand-options" role="radiogroup" aria-label="Quais">
      {[['', 'Todos'], ['ayla', 'Criados pela Ayla'], ['desativado', 'Desativados']].map(([v, l]) => <label key={v} className={fonte === v ? 'hand-chip on' : 'hand-chip'}><input type="radio" name="hand-fonte" checked={fonte === v} onChange={() => setFonte(v)} />{l}</label>)}
      <label className="hand-check"><input type="checkbox" checked={internos} onChange={e => setInternos(e.target.checked)} />Mostrar internos</label>
    </div>
    {notice && <p role="status" className="hand-done">{notice}</p>}
    {creating && <NewShortcut change={change} close={() => setCreating(false)} />}
    {!list ? <p role="status" className="panel-muted">Lendo a configuração…</p> : !shown.length ? <p className="panel-muted">Nenhum atalho assim.</p>
      : groups.map(([name, items]) => <section key={name} className="hand-bind-group">
        <h3>{name}</h3>
        <ul className="hand-binds">{collapse(items, internos).map(entry => 'members' in entry
          ? <li key={entry.grupo.id} className="hand-bind-summary">
              <Keys combo={entry.grupo.combo} />
              <span className="hand-bind-what"><b>{entry.grupo.descricao}</b><small>{entry.members.length} atalhos · {entry.members.filter(m => m.estado !== 'ativo').length ? `${entry.members.filter(m => m.estado !== 'ativo').length} mudados · ` : ''}iguais, um por tecla</small></span>
              <span className="row-actions"><button className="ghost-button" aria-expanded={open.includes(entry.grupo.id)} onClick={() => setOpen(prev => prev.includes(entry.grupo.id) ? prev.filter(x => x !== entry.grupo.id) : [...prev, entry.grupo.id])}>{open.includes(entry.grupo.id) ? 'Fechar' : 'Ver cada um'}</button></span>
              {open.includes(entry.grupo.id) && <ul className="hand-binds hand-bind-members">{entry.members.map(row)}</ul>}
            </li>
          : row(entry))}</ul>
      </section>)}
  </>;
}

function Conflict({ outro, onReplace }: { outro: Atalho; onReplace: () => void }) {
  return <p className="app-error hand-conflict"><span>{comboLabel(outro.combo).join('+')} já é “{describe(outro) || actionLabel(outro)}”.</span> <button className="ghost-button" onClick={onReplace}>Usar mesmo assim (desativa o outro)</button></p>;
}

function ChangeKey({ atalho, change, close }: { atalho: Atalho; change: (op: Record<string, unknown>, ok: string) => Promise<Resposta>; close: () => void }) {
  const [combo, setCombo] = useState<Combo>(atalho.combo);
  const [conflito, setConflito] = useState<Atalho | null>(null);
  async function save(substituir = false) {
    const r = await change({ op: 'trocar', id: atalho.id, mods: combo.mods.join(' '), tecla: combo.tecla, substituir }, `Agora é ${comboLabel(combo).join('+')}.`);
    if (r.conflito) setConflito(r.conflito); else if (r.ok) close();
  }
  return <div className="hand-bind-edit">
    <ComboInput value={combo} onChange={c => { setCombo(c); setConflito(null); }} />
    {conflito && <Conflict outro={conflito} onReplace={() => void save(true)} />}
    <div className="row-actions"><button className="primary-action" disabled={!combo.tecla} onClick={() => void save()}>Salvar</button><button className="ghost-button" onClick={close}>Cancelar</button></div>
  </div>;
}

function NewShortcut({ change, close }: { change: (op: Record<string, unknown>, ok: string) => Promise<Resposta>; close: () => void }) {
  const [combo, setCombo] = useState<Combo>({ mods: ['SUPER'], tecla: '' });
  const [kind, setKind] = useState<'comando' | 'programa' | 'hyprland'>('comando');
  const [descricao, setDescricao] = useState('');
  const [arg, setArg] = useState('');
  const [dispatcher, setDispatcher] = useState('');
  const [programas, setProgramas] = useState<Programa[]>([]);
  const [conflito, setConflito] = useState<Atalho | null>(null);
  useEffect(() => { if (kind === 'programa' && !programas.length) void native<Programa[]>('hand_programas').then(setProgramas).catch(() => {}); }, [kind, programas.length]);
  async function save(substituir = false) {
    const acao = kind !== 'hyprland' ? 'exec' : dispatcher.trim();
    const args = kind === 'programa' ? `gtk-launch ${arg}` : arg;
    const desc = descricao.trim() || (kind === 'programa' ? `Abrir ${programas.find(p => p.id === arg)?.name ?? arg}` : '');
    const r = await change({ op: 'criar', mods: combo.mods.join(' '), tecla: combo.tecla, descricao: desc, acao, arg: args, substituir }, `Atalho ${comboLabel(combo).join('+')} criado.`);
    if (r.conflito) setConflito(r.conflito); else if (r.ok) close();
  }
  return <section className="hand-plan hand-new-bind" aria-label="Novo atalho">
    <ComboInput value={combo} onChange={c => { setCombo(c); setConflito(null); }} />
    <div className="hand-search">
      <select aria-label="O que faz" value={kind} onChange={e => { setKind(e.target.value as typeof kind); setArg(''); }}>
        <option value="comando">Rodar um comando</option><option value="programa">Abrir um programa</option><option value="hyprland">Ação do Hyprland (avançado)</option>
      </select>
      {kind === 'programa'
        ? <select aria-label="Programa" value={arg} onChange={e => setArg(e.target.value)}><option value="">Escolha…</option>{programas.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}</select>
        : <>{kind === 'hyprland' && <input className="hand-command hand-dispatcher" value={dispatcher} placeholder="ação (ex.: workspace)" aria-label="Ação do Hyprland" spellCheck={false} onChange={e => setDispatcher(e.target.value)} />}
          <input className="hand-command" value={arg} placeholder={kind === 'comando' ? 'comando (ex.: kitty -e htop)' : 'argumento (ex.: 3)'} aria-label="Comando" spellCheck={false} onChange={e => setArg(e.target.value)} /></>}
    </div>
    <input value={descricao} placeholder="Descrição (aparece na lista e na cola de atalhos)" aria-label="Descrição" onChange={e => setDescricao(e.target.value)} />
    {conflito && <Conflict outro={conflito} onReplace={() => void save(true)} />}
    <div className="row-actions"><button className="primary-action" disabled={!combo.tecla || !arg.trim() || (kind === 'hyprland' && !dispatcher.trim())} onClick={() => void save()}>Criar atalho</button><button className="ghost-button" onClick={close}>Cancelar</button></div>
  </section>;
}

// ---------------------------------------------------------------------------------------------
// Macros

const newId = () => `m-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

function stepLabel(p: Passo): [string, string] {
  switch (p.tipo) {
    case 'digitar': return ['Digitar', `“${p.texto}”`];
    case 'teclas': return ['Apertar', p.atalho];
    case 'gravado': return ['Teclas', p.rotulo || '(teclas gravadas)'];
    case 'clicar': return [p.vezes > 1 ? 'Clique duplo' : p.botao === 'direito' ? 'Clique direito' : p.botao === 'meio' ? 'Clique do meio' : 'Clicar', `em ${p.x}, ${p.y}`];
    case 'arrastar': return ['Arrastar', `de ${p.de.join(', ')} até ${p.para.join(', ')}`];
    case 'rolar': return ['Rolar', `${Math.abs(p.dentes)} ${p.dentes > 0 ? 'para baixo' : 'para cima'}`];
    case 'esperar': return ['Esperar', `${(p.ms / 1000).toLocaleString('pt-BR')} s`];
    case 'abrir': return ['Abrir', p.nome];
  }
}

function Macros({ atalhos, reloadShortcuts, fail, active }: { atalhos: Atalho[]; reloadShortcuts: () => Promise<void>; fail: (e: unknown) => void; active: boolean }) {
  const [state, setState] = useState<{ macros: Macro[]; gravando: boolean; falta: string | null; binario: string } | null>(null);
  const [editing, setEditing] = useState<Macro | null>(null);
  const [notice, setNotice] = useState('');
  const load = useCallback(() => native<{ macros: Macro[]; gravando: boolean; falta: string | null; binario: string }>('hand_macros').then(setState).catch(fail), [fail]);
  useEffect(() => { if (active) void load(); }, [active, load]);
  const shortcutOf = (id: string) => atalhos.find(a => a.fonte === 'ayla' && a.arg.endsWith(`--macro ${id}`));
  async function run(m: Macro) {
    setNotice(`Rodando “${m.nome}” em 3 s: vá até a janela certa.`);
    try { await native('hand_macro_rodar', { id: m.id, espera: 3 }); setNotice(`“${m.nome}” terminou.`); }
    catch (e) { setNotice(''); fail(e); }
  }
  if (editing) return <MacroEditor initial={editing} binario={state?.binario ?? 'ayla'} falta={state?.falta ?? null} shortcut={shortcutOf(editing.id)} fail={fail}
    close={saved => { setEditing(null); if (saved) { setNotice(`Macro “${saved}” salva.`); void load(); void reloadShortcuts(); } }} />;
  return <>
    <p className="panel-muted">Uma macro repete teclado e mouse: monte passo a passo ou grave o que você faz. Rode pelo botão ou por um atalho.</p>
    <div className="row-actions"><button className="primary-action" onClick={() => setEditing({ id: newId(), nome: '', passos: [] })}><PlusIcon size={15} />Nova macro</button></div>
    {notice && <p role="status" className="hand-done">{notice}</p>}
    {state && !state.macros.length && <p className="panel-muted">Nenhuma macro ainda.</p>}
    <ul className="hand-list">{state?.macros.map(m => {
      const s = shortcutOf(m.id);
      return <li key={m.id}>
        <span className="hand-name"><b>{m.nome}</b><small>{m.passos.length} {m.passos.length === 1 ? 'passo' : 'passos'}</small>{s && <Keys combo={s.combo} />}</span>
        <span className="row-actions">
          <button className="ghost-button" onClick={() => void run(m)}>Rodar</button>
          <button className="ghost-button" onClick={() => setEditing(m)}>Editar</button>
          <Confirm className="icon-button" ariaLabel={`Apagar ${m.nome}`} title="Apagar macro" question={`Apagar a macro “${m.nome}”${s ? ' e o atalho dela' : ''}?`} confirm="Apagar"
            onConfirm={() => void native('hand_macro_apagar', { id: m.id }).then(() => { setNotice('Macro apagada.'); void load(); void reloadShortcuts(); }).catch(fail)}><TrashIcon size={15} /></Confirm>
        </span>
      </li>;
    })}</ul>
  </>;
}

function MacroEditor({ initial, binario, falta, shortcut, fail, close }: { initial: Macro; binario: string; falta: string | null; shortcut?: Atalho; fail: (e: unknown) => void; close: (saved?: string) => void }) {
  const [nome, setNome] = useState(initial.nome);
  const [passos, setPassos] = useState<Passo[]>(initial.passos);
  const [recording, setRecording] = useState(false);
  const [combo, setCombo] = useState<Combo>(shortcut?.combo ?? { mods: ['SUPER', 'ALT'], tecla: '' });
  const [conflito, setConflito] = useState<Atalho | null>(null);
  const [busy, setBusy] = useState('');
  const [programas, setProgramas] = useState<Programa[]>([]);
  useEffect(() => { void native<Programa[]>('hand_programas').then(setProgramas).catch(() => {}); }, []);
  const set = (i: number, p: Passo) => setPassos(prev => prev.map((x, j) => j === i ? p : x));
  const move = (i: number, d: number) => setPassos(prev => { const next = [...prev]; const j = i + d; if (j < 0 || j >= next.length) return prev; [next[i], next[j]] = [next[j], next[i]]; return next; });
  async function record() {
    if (recording) {
      try { const got = await native<Passo[]>('hand_gravar_parar'); setPassos(prev => [...prev, ...got]); } catch (e) { fail(e); }
      setRecording(false);
      return;
    }
    try { await native('hand_gravar_comecar'); setRecording(true); } catch (e) { fail(e); }
  }
  async function pick(i: number) {
    const p = passos[i];
    if (p.tipo !== 'clicar') return;
    setBusy(`Leve o mouse até o ponto: pego a posição em 3 s…`);
    try { const [x, y] = await native<[number, number]>('hand_cursor', { espera: 3 }); set(i, { ...p, x, y }); } catch (e) { fail(e); }
    setBusy('');
  }
  async function save(substituir = false) {
    const item: Macro = { id: initial.id, nome: nome.trim(), passos };
    try {
      await native('hand_macro_salvar', { item });
      if (combo.tecla && (!shortcut || comboLabel(shortcut.combo).join('+') !== comboLabel(combo).join('+'))) {
        const op = shortcut ? { op: 'trocar', id: shortcut.id, mods: combo.mods.join(' '), tecla: combo.tecla, substituir }
          : { op: 'criar', mods: combo.mods.join(' '), tecla: combo.tecla, descricao: `Macro: ${item.nome}`, acao: 'exec', arg: `${binario} --macro ${item.id}`, substituir };
        const r = await native<Resposta>('hand_atalho_mudar', { op });
        if (r.conflito) { setConflito(r.conflito); return; }
      } else if (!combo.tecla && shortcut) {
        await native('hand_atalho_mudar', { op: { op: 'desativar', id: shortcut.id } });
      }
      close(item.nome);
    } catch (e) { fail(e); }
  }
  const add = (p: Passo) => setPassos(prev => [...prev, p]);
  return <section className="hand-plan hand-macro-editor" aria-label="Macro">
    <input value={nome} autoFocus placeholder="Nome da macro (ex.: Assinatura do e-mail)" aria-label="Nome da macro" onChange={e => setNome(e.target.value)} />
    <ol className="hand-macro-steps">{passos.map((p, i) => {
      const [verb, what] = stepLabel(p);
      return <li key={i}>
        <span className="hand-verb">{verb}</span>
        {p.tipo === 'digitar' ? <input value={p.texto} aria-label="Texto" onChange={e => set(i, { ...p, texto: e.target.value })} />
          : p.tipo === 'teclas' ? <input className="hand-command" value={p.atalho} aria-label="Combinação" placeholder="ctrl+shift+t" spellCheck={false} onChange={e => set(i, { ...p, atalho: e.target.value })} />
          : p.tipo === 'esperar' ? <label className="hand-inline"><input className="hand-number" type="number" min={0.1} max={600} step={0.1} value={p.ms / 1000} onChange={e => set(i, { ...p, ms: Math.round(Number(e.target.value) * 1000) })} />segundos</label>
          : p.tipo === 'rolar' ? <label className="hand-inline"><input className="hand-number" type="number" min={-50} max={50} value={p.dentes} onChange={e => set(i, { ...p, dentes: Number(e.target.value) })} />(positivo desce)</label>
          : p.tipo === 'clicar' ? <span className="hand-inline">
              <select aria-label="Botão" value={p.botao} onChange={e => set(i, { ...p, botao: e.target.value as Botao })}><option value="esquerdo">esquerdo</option><option value="direito">direito</option><option value="meio">meio</option></select>
              <select aria-label="Vezes" value={p.vezes} onChange={e => set(i, { ...p, vezes: Number(e.target.value) })}><option value={1}>1×</option><option value={2}>2×</option><option value={3}>3×</option></select>
              <input className="hand-number" type="number" aria-label="x" value={p.x} onChange={e => set(i, { ...p, x: Number(e.target.value) })} />
              <input className="hand-number" type="number" aria-label="y" value={p.y} onChange={e => set(i, { ...p, y: Number(e.target.value) })} />
              <button type="button" className="ghost-button" disabled={!!busy} onClick={() => void pick(i)}>Pegar posição</button></span>
          : p.tipo === 'abrir' ? <select aria-label="Programa" value={p.programa} onChange={e => set(i, { ...p, programa: e.target.value, nome: programas.find(x => x.id === e.target.value)?.name ?? e.target.value })}>
              <option value="">Escolha…</option>{programas.map(x => <option key={x.id} value={x.id}>{x.name}</option>)}</select>
          : <span className="hand-step-text">{what}</span>}
        <span className="row-actions">
          <button type="button" className="icon-button" aria-label="Subir passo" onClick={() => move(i, -1)} disabled={i === 0}>↑</button>
          <button type="button" className="icon-button" aria-label="Descer passo" onClick={() => move(i, 1)} disabled={i === passos.length - 1}>↓</button>
          <button type="button" className="icon-button" aria-label="Tirar passo" onClick={() => setPassos(prev => prev.filter((_, j) => j !== i))}><TrashIcon size={14} /></button>
        </span>
      </li>;
    })}</ol>
    {!passos.length && <p className="panel-muted">Sem passos ainda: adicione abaixo ou grave.</p>}
    <div className="row-actions hand-add-steps">
      <button type="button" className="ghost-button" onClick={() => add({ tipo: 'digitar', texto: '' })}>+ Digitar texto</button>
      <button type="button" className="ghost-button" onClick={() => add({ tipo: 'teclas', atalho: 'ctrl+c' })}>+ Combinação</button>
      <button type="button" className="ghost-button" onClick={() => add({ tipo: 'clicar', botao: 'esquerdo', x: 0, y: 0, vezes: 1 })}>+ Clique</button>
      <button type="button" className="ghost-button" onClick={() => add({ tipo: 'rolar', dentes: 3 })}>+ Rolar</button>
      <button type="button" className="ghost-button" onClick={() => add({ tipo: 'esperar', ms: 1000 })}>+ Esperar</button>
      <button type="button" className="ghost-button" onClick={() => add({ tipo: 'abrir', programa: '', nome: '' })}>+ Abrir programa</button>
      <button type="button" className={recording ? 'danger-action' : 'ghost-button'} disabled={!!falta && !recording} onClick={() => void record()}>{recording ? '■ Parar gravação' : '● Gravar'}</button>
    </div>
    {recording && <p role="status" className="hand-recording">Gravando teclado e mouse… Faça os passos e clique em Parar gravação. O clique no Parar não entra.</p>}
    {falta && <p className="panel-muted">{falta.split('`').map((part, i) => i % 2 ? <code key={i}>{part}</code> : part)}</p>}
    {busy && <p role="status" className="panel-muted">{busy}</p>}
    <div className="hand-macro-key"><span className="hand-options-label">Atalho para rodar (opcional)</span><ComboInput value={combo} onChange={c => { setCombo(c); setConflito(null); }} />
      {combo.tecla && <button type="button" className="link-button" onClick={() => setCombo({ mods: [], tecla: '' })}>Sem atalho</button>}</div>
    {conflito && <Conflict outro={conflito} onReplace={() => void save(true)} />}
    <div className="row-actions">
      <button className="primary-action" disabled={!nome.trim() || !passos.length || recording} onClick={() => void save()}>Salvar macro</button>
      <button className="ghost-button" disabled={recording} onClick={() => close()}>Cancelar</button>
    </div>
  </section>;
}


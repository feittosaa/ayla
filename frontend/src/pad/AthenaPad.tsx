import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { type LanguageSupport } from '@codemirror/language';
import { gotoLine, openSearchPanel } from '@codemirror/search';
import { EditorSelection, EditorState, Text } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { errorText, native, type Snapshot } from '../services/Desktop';
import { DEFAULT_APPEARANCE, applyAppearance, useMode, type Appearance } from '../services/Appearance';
import { CloseIcon, FileIcon } from '../components/Icons';
import BrandMark from '../components/BrandMark';
import { DEFAULTS, configure, extensions, languageFor, languageNames, type Settings } from './editor';
import { changeCase, joinLines, position, remember, sortLines, untitledName, type CaseMode } from './textOps';
import './pad.css';

type Note = { root: string; path: string };
type Tab = {
  id: string; title: string; path: string | null; note: Note | null; encoding: string; eol: '\n' | '\r\n';
  modified: number | null; language: string | null; readonly: boolean; changedOnDisk: boolean; missing: boolean;
};
type Doc = { path: string; name: string; text: string; encoding: string; eol: '\n' | '\r\n'; modified: number; readonly: boolean };
type SavedTab = Omit<Tab, 'changedOnDisk' | 'missing'> & { draft: string | null };
type Session = { tabs?: SavedTab[]; active?: string; recent?: string[]; settings?: Partial<Settings> };
type NoteNode = { root: string; rootName: string; path: string; title: string };
type Closing = { id: string; discard: boolean } | null;

const newId = () => crypto.randomUUID();
// Pasta com ~ no lugar da pasta pessoal, como no Text Editor.
const homeShort = (folder: string | null) => folder?.replace(/^\/home\/[^/]+(?=\/|$)/, '~') ?? null;

const Svg = ({ children }: { children: React.ReactNode }) => <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{children}</svg>;
const ChevronDown = () => <Svg><path d="M4.5 6.5 8 10l3.5-3.5" /></Svg>;
const NewTabIcon = () => <Svg><rect x="2.5" y="2.5" width="11" height="11" rx="2" /><path d="M8 5.5v5M5.5 8h5" /></Svg>;
const InfoIcon = () => <Svg><circle cx="8" cy="8" r="6" /><path d="M8 7.2V11M8 5v.01" /></Svg>;
const MenuIcon = () => <Svg><path d="M2.5 4h11M2.5 8h11M2.5 12h11" /></Svg>;
const folderOf = (path: string | null) => path ? path.slice(0, path.lastIndexOf('/')) || '/' : null;

/** Athena sozinha, como bloco de notas: abas, arquivos do computador e notas da Athena. */
export default function AthenaPad() {
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [dirty, setDirty] = useState<Record<string, boolean>>({});
  const [settings, setSettings] = useState<Settings>(DEFAULTS);
  const [recent, setRecent] = useState<string[]>([]);
  const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const [menu, setMenu] = useState<'main' | 'open' | 'info' | null>(null);
  const [panel, setPanel] = useState<'prefs' | 'notes' | null>(null);
  const [closing, setClosing] = useState<Closing>(null);
  const [conflict, setConflict] = useState<string | null>(null);
  const [cursor, setCursor] = useState({ line: 1, col: 1, selected: 0 });
  const [printing, setPrinting] = useState<string | null>(null);
  const [look, setLook] = useState<Appearance>(DEFAULT_APPEARANCE);
  const [ready, setReady] = useState(false);
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const states = useRef(new Map<string, EditorState>());
  const saved = useRef(new Map<string, Text>());
  const languages = useRef(new Map<string, LanguageSupport | null>());
  const tabsRef = useRef(tabs); tabsRef.current = tabs;
  const activeRef = useRef(active); activeRef.current = active;
  const settingsRef = useRef(settings); settingsRef.current = settings;
  const recentRef = useRef(recent); recentRef.current = recent;
  const [revision, setRevision] = useState(0);
  // Arrastar abas para reordenar: só vira arraste depois de 5 px (antes disso é clique).
  const tabsNav = useRef<HTMLElement>(null);
  const drag = useRef<{ id: string; x: number; moved: boolean } | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);

  // Aparência da Ayla (fundo, superfícies, texto e fontes), com o modo claro ou escuro.
  const mode = useMode(look);
  useLayoutEffect(() => { applyAppearance(look, mode); document.documentElement.dataset.mode = mode; }, [look, mode]);
  // A aparência salva ao abrir e ao voltar o foco; mudanças na Ayla (e a prévia) chegam pelo aviso `ayla-appearance`.
  useEffect(() => {
    const load = () => void native<Snapshot>('desktop_snapshot').then(s => setLook(s.preferences.appearance)).catch(() => {});
    load(); window.addEventListener('focus', load);
    const off = listen<Appearance>('ayla-appearance', event => setLook(event.payload));
    return () => { window.removeEventListener('focus', load); void off.then(f => f()); };
  }, []);

  const tabOf = (id: string | null) => tabsRef.current.find(t => t.id === id) ?? null;
  const textOf = (id: string) => (id === activeRef.current && view.current ? view.current.state : states.current.get(id))?.doc.toString() ?? '';
  const isDirty = (id: string) => {
    const state = id === activeRef.current && view.current ? view.current.state : states.current.get(id);
    const base = saved.current.get(id);
    return Boolean(state && (!base || !state.doc.eq(base)));
  };
  const refreshDirty = useCallback(() => setDirty(Object.fromEntries(tabsRef.current.map(t => [t.id, isDirty(t.id)]))), []);
  const patchTab = (id: string, change: Partial<Tab>) => { tabsRef.current = tabsRef.current.map(t => t.id === id ? { ...t, ...change } : t); setTabs(tabsRef.current); };

  // Linguagem do destaque: carregada sob demanda e guardada por aba.
  async function loadLanguage(tab: Tab) {
    const description = languageFor(tab.path ?? (tab.note ? tab.note.path : tab.title), tab.language);
    const support = description ? await description.load().catch(() => null) : null;
    languages.current.set(tab.id, support);
    return support;
  }
  const applyConfig = useCallback((id: string) => {
    const editor = view.current; const tab = tabOf(id);
    if (!editor || !tab) return;
    editor.dispatch({ effects: configure(settingsRef.current, languages.current.get(id) ?? null, tab.readonly).map(([slot, value]) => slot.reconfigure(value)) });
  }, []);

  function stateFor(tab: Tab, text: string) {
    return EditorState.create({ doc: text, extensions: extensions(settingsRef.current, languages.current.get(tab.id) ?? null, tab.readonly, update => {
      if (update.docChanged) { setRevision(r => r + 1); }
      if (update.docChanged || update.selectionSet) {
        const head = update.state.selection.main.head; const line = update.state.doc.lineAt(head);
        setCursor({ line: line.number, col: position(line.text, head - line.from, update.state.tabSize), selected: update.state.selection.ranges.reduce((n, r) => n + r.to - r.from, 0) });
      }
    }) });
  }

  /** Acrescenta uma aba (texto atual `text`, texto salvo `base`) e a ativa. */
  async function addTab(tab: Tab, text: string, base: string | null, activate = true) {
    await loadLanguage(tab);
    states.current.set(tab.id, stateFor(tab, text));
    if (base !== null) saved.current.set(tab.id, Text.of(base.split('\n'))); else saved.current.delete(tab.id);
    tabsRef.current = [...tabsRef.current, tab]; setTabs(tabsRef.current);
    if (activate) setActive(tab.id);
  }
  function newDocument() {
    const tab: Tab = { id: newId(), title: untitledName(tabsRef.current.map(t => t.title)), path: null, note: null, encoding: 'UTF-8', eol: '\n', modified: null, language: null, readonly: false, changedOnDisk: false, missing: false };
    void addTab(tab, '', '');
  }
  async function openPath(path: string, activate = true) {
    const open = tabsRef.current.find(t => t.path === path);
    if (open) { if (activate) setActive(open.id); return; }
    try {
      const doc = await native<Doc>('pad_read', { path });
      await addTab({ id: newId(), title: doc.name, path: doc.path, note: null, encoding: doc.encoding, eol: doc.eol, modified: doc.modified, language: null, readonly: doc.readonly, changedOnDisk: false, missing: false }, doc.text, doc.text, activate);
      setRecent(prev => remember(prev, doc.path));
    } catch (e) { setError(`${path.split('/').pop()}: ${errorText(e)}`); }
  }
  async function openNote(note: NoteNode) {
    setPanel(null);
    const open = tabsRef.current.find(t => t.note?.root === note.root && t.note.path === note.path);
    if (open) { setActive(open.id); return; }
    try {
      const read = await native<{ content: string; modified: number }>('athena_read', { root: note.root, path: note.path });
      await addTab({ id: newId(), title: note.title, path: null, note: { root: note.root, path: note.path }, encoding: 'UTF-8', eol: '\n', modified: read.modified, language: 'Markdown', readonly: false, changedOnDisk: false, missing: false }, read.content, read.content);
    } catch (e) { setError(`${note.title}: ${errorText(e)}`); }
  }
  async function openDialog() {
    setMenu(null); setPanel(null);
    try { for (const path of await native<string[]>('pad_open_dialog')) await openPath(path); } catch (e) { setError(errorText(e)); }
  }

  // Salvar: grava e confere se o arquivo mudou no disco desde que foi aberto.
  async function save(id: string, force = false): Promise<boolean> {
    const tab = tabOf(id); if (!tab) return false;
    if (!tab.path && !tab.note) return saveAs(id);
    const text = textOf(id);
    try {
      const stamp = tab.note
        ? await native<number>('athena_write', { root: tab.note.root, path: tab.note.path, content: text, expected: force ? null : tab.modified })
        : await native<number>('pad_write', { path: tab.path, text, encoding: tab.encoding, eol: tab.eol, expected: force || tab.missing ? null : tab.modified });
      saved.current.set(id, Text.of(text.split('\n')));
      patchTab(id, { modified: stamp, changedOnDisk: false, missing: false }); setConflict(null); refreshDirty();
      return true;
    } catch (e) {
      const message = errorText(e);
      if (/mudou/i.test(message)) setConflict(id); else setError(message);
      return false;
    }
  }
  async function saveAs(id: string): Promise<boolean> {
    const tab = tabOf(id); if (!tab) return false;
    const name = tab.path ? tab.path.split('/').pop()! : tab.note ? tab.note.path.split('/').pop()! : `${tab.title}.txt`;
    try {
      const path = await native<string | null>('pad_save_dialog', { name, folder: folderOf(tab.path) ?? folderOf(recentRef.current[0] ?? null) });
      if (!path) return false;
      const text = textOf(id);
      const stamp = await native<number>('pad_write', { path, text, encoding: tab.encoding, eol: tab.eol, expected: null });
      saved.current.set(id, Text.of(text.split('\n')));
      const updated = { ...tab, path, note: null, title: path.split('/').pop()!, modified: stamp, language: null, missing: false, changedOnDisk: false };
      patchTab(id, updated); setRecent(prev => remember(prev, path));
      await loadLanguage(updated); if (id === activeRef.current) applyConfig(id);
      refreshDirty();
      return true;
    } catch (e) { setError(errorText(e)); return false; }
  }
  async function reload(id: string) {
    const tab = tabOf(id); if (!tab) return;
    try {
      const doc = tab.note
        ? await native<{ content: string; modified: number }>('athena_read', { root: tab.note.root, path: tab.note.path }).then(r => ({ text: r.content, modified: r.modified }))
        : await native<Doc>('pad_read', { path: tab.path });
      const editor = view.current;
      if (id === activeRef.current && editor) editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: doc.text } });
      else { const state = states.current.get(id); if (state) states.current.set(id, state.update({ changes: { from: 0, to: state.doc.length, insert: doc.text } }).state); }
      saved.current.set(id, Text.of(doc.text.split('\n')));
      patchTab(id, { modified: doc.modified, changedOnDisk: false, missing: false }); setConflict(null); refreshDirty();
    } catch (e) { setError(errorText(e)); }
  }

  function moveTab(id: string, to: number) {
    const list = [...tabsRef.current]; const from = list.findIndex(t => t.id === id);
    if (from < 0 || from === to) return;
    const [moved] = list.splice(from, 1); list.splice(to, 0, moved);
    tabsRef.current = list; setTabs(list);
  }
  function dragStart(event: React.PointerEvent<HTMLDivElement>, id: string) {
    if (event.button !== 0 || (event.target as HTMLElement).closest('.pad-tab-close')) return;
    drag.current = { id, x: event.clientX, moved: false };
  }
  function dragMove(event: React.PointerEvent<HTMLDivElement>, id: string) {
    const current = drag.current; if (!current || current.id !== id) return;
    if (!current.moved) {
      if (Math.abs(event.clientX - current.x) < 5) return;
      current.moved = true; setDragging(id); setActive(id);
      try { event.currentTarget.setPointerCapture(event.pointerId); } catch { /* ponteiro já solto */ }
    }
    // Posição nova: quantas das outras abas têm o centro à esquerda do ponteiro.
    const others = [...(tabsNav.current?.children ?? [])].filter(el => (el as HTMLElement).dataset.id !== id);
    moveTab(id, others.filter(el => { const r = el.getBoundingClientRect(); return r.left + r.width / 2 < event.clientX; }).length);
  }
  function dragEnd() { drag.current = null; setDragging(null); }

  // Fechar: com alterações, pergunta; descartar pede uma segunda confirmação.
  function requestClose(id: string) { if (isDirty(id)) { setActive(id); setClosing({ id, discard: false }); } else closeTab(id); }
  function closeTab(id: string) {
    setClosing(null);
    const list = tabsRef.current; const index = list.findIndex(t => t.id === id);
    const rest = list.filter(t => t.id !== id);
    states.current.delete(id); saved.current.delete(id); languages.current.delete(id);
    tabsRef.current = rest; setTabs(rest);
    if (activeRef.current === id) setActive(rest[Math.min(index, rest.length - 1)]?.id ?? null);
    if (!rest.length) setTimeout(newDocument, 0);
  }

  // Troca de aba: guarda o estado da anterior (com o histórico) e mostra o da nova.
  useLayoutEffect(() => {
    if (!host.current) return;
    if (!view.current) view.current = new EditorView({ parent: host.current });
    const editor = view.current;
    const previous = editor.dom.dataset.tab;
    if (previous && previous !== active && states.current.has(previous)) states.current.set(previous, editor.state);
    if (active && states.current.has(active) && previous !== active) {
      editor.setState(states.current.get(active)!);
      editor.dom.dataset.tab = active;
      applyConfig(active);
      editor.focus();
    }
  }, [active, applyConfig]);
  useEffect(() => () => { view.current?.destroy(); view.current = null; }, []);
  // Preferências mudaram: vale para a aba aberta já, e para as outras ao abri-las.
  useEffect(() => { if (activeRef.current) applyConfig(activeRef.current); }, [settings, applyConfig]);
  useEffect(() => { const timer = setTimeout(refreshDirty, 250); return () => clearTimeout(timer); }, [revision, refreshDirty]);

  // Sessão: abas, rascunhos não salvos, recentes e preferências, guardados a cada pausa.
  useEffect(() => {
    if (!ready) return;
    const timer = setTimeout(() => {
      if (activeRef.current && view.current) states.current.set(activeRef.current, view.current.state);
      const session: Session = {
        active: activeRef.current ?? undefined, recent: recentRef.current, settings: settingsRef.current,
        tabs: settingsRef.current.restoreSession ? tabsRef.current.map(t => ({ ...t, draft: isDirty(t.id) || (!t.path && !t.note) ? textOf(t.id) : null })) : [],
      };
      void native('pad_session_save', { session }).catch(e => setError(errorText(e)));
    }, 1500);
    return () => clearTimeout(timer);
  }, [ready, tabs, active, recent, settings, revision]);

  // Abertura: restaura a sessão e abre os arquivos pedidos pelo "Abrir com".
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return; started.current = true;
    void (async () => {
      const session = await native<Session | null>('pad_session_load').catch(() => null);
      const restored = { ...DEFAULTS, ...(session?.settings ?? {}) };
      settingsRef.current = restored; setSettings(restored); setRecent(session?.recent ?? []);
      if (restored.restoreSession) {
        for (const saved of session?.tabs ?? []) {
          const tab: Tab = { ...saved, changedOnDisk: false, missing: false };
          if (tab.path) {
            const doc = await native<Doc>('pad_read', { path: tab.path }).catch(() => null);
            if (doc) await addTab({ ...tab, modified: doc.modified, readonly: doc.readonly, encoding: doc.encoding, eol: doc.eol }, saved.draft ?? doc.text, doc.text, false);
            else if (saved.draft !== null) await addTab({ ...tab, missing: true }, saved.draft, null, false);
          } else if (tab.note) {
            const read = await native<{ content: string; modified: number }>('athena_read', { root: tab.note.root, path: tab.note.path }).catch(() => null);
            if (read) await addTab({ ...tab, modified: read.modified }, saved.draft ?? read.content, read.content, false);
          } else await addTab(tab, saved.draft ?? '', '', false);
        }
      }
      const restoredTabs = tabsRef.current;
      setActive(restoredTabs.find(t => t.id === session?.active)?.id ?? restoredTabs[restoredTabs.length - 1]?.id ?? null);
      for (const path of await native<string[]>('pad_requests').catch(() => [])) await openPath(path);
      if (!tabsRef.current.length) newDocument();
      setReady(true); refreshDirty();
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    const off = listen('athena-open', () => { void native<string[]>('pad_requests').then(async paths => { for (const path of paths) await openPath(path); }); });
    return () => { void off.then(f => f()); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Arquivo mudou fora do editor: sem alterações, recarrega; com alterações, pergunta.
  useEffect(() => {
    const check = () => {
      const tab = tabOf(activeRef.current);
      if (!tab?.path || tab.missing || !document.hasFocus()) return;
      void native<number | null>('pad_modified', { path: tab.path }).then(stamp => {
        if (stamp === null) patchTab(tab.id, { missing: true });
        else if (tab.modified !== null && stamp !== tab.modified) { if (isDirty(tab.id)) patchTab(tab.id, { changedOnDisk: true }); else void reload(tab.id); }
      }).catch(() => {});
    };
    window.addEventListener('focus', check); const timer = setInterval(check, 3000);
    return () => { window.removeEventListener('focus', check); clearInterval(timer); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Comandos sobre a seleção (ou o documento inteiro).
  function transform(change: (text: string) => string, whole = false) {
    setMenu(null);
    const editor = view.current; if (!editor) return;
    const ranges = editor.state.selection.ranges.filter(r => !r.empty);
    const targets = whole || !ranges.length ? [{ from: 0, to: editor.state.doc.length }] : ranges.map(r => ({ from: editor.state.doc.lineAt(r.from).from, to: editor.state.doc.lineAt(r.to).to }));
    editor.dispatch({ changes: targets.map(t => ({ ...t, insert: change(editor.state.sliceDoc(t.from, t.to)) })) });
    editor.focus();
  }
  function caseChange(mode: CaseMode) {
    setMenu(null);
    const editor = view.current; if (!editor) return;
    editor.dispatch(editor.state.changeByRange(range => {
      const from = range.empty ? editor.state.wordAt(range.head)?.from ?? range.from : range.from;
      const to = range.empty ? editor.state.wordAt(range.head)?.to ?? range.to : range.to;
      const insert = changeCase(editor.state.sliceDoc(from, to), mode);
      return { changes: { from, to, insert }, range: EditorSelection.range(from, from + insert.length) };
    }));
    editor.focus();
  }
  function print() {
    setMenu(null);
    const tab = tabOf(activeRef.current); if (!tab) return;
    setPrinting(textOf(tab.id));
    setTimeout(() => { window.print(); setPrinting(null); }, 50);
  }
  const zoom = (step: number) => setSettings(s => ({ ...s, zoom: step === 0 ? 1 : Math.min(3, Math.max(0.5, Math.round((s.zoom + step) * 10) / 10)) }));

  // Menu aberto: clicar fora dele (e fora do botão que o abre) fecha.
  useEffect(() => {
    if (!menu) return;
    const outside = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null;
      if (!target?.closest('.pad-menu, [aria-expanded=true]')) setMenu(null);
    };
    window.addEventListener('pointerdown', outside, true);
    return () => window.removeEventListener('pointerdown', outside, true);
  }, [menu]);

  // Atalhos do Text Editor fora do texto (abas, arquivos, zoom, impressão).
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      const ctrl = event.ctrlKey || event.metaKey; const key = event.key.toLowerCase(); const id = activeRef.current;
      const handled = () => { event.preventDefault(); event.stopPropagation(); };
      if (ctrl && !event.shiftKey && key === 'n') { handled(); newDocument(); }
      else if (ctrl && key === 't') { handled(); newDocument(); }
      else if (ctrl && key === 'o') { handled(); void openDialog(); }
      else if (ctrl && event.shiftKey && key === 's') { handled(); if (id) void saveAs(id); }
      else if (ctrl && key === 's') { handled(); if (id) void save(id); }
      else if (ctrl && key === 'w') { handled(); if (id) requestClose(id); }
      else if (ctrl && key === 'p') { handled(); print(); }
      else if (ctrl && (key === '+' || key === '=')) { handled(); zoom(0.1); }
      else if (ctrl && key === '-') { handled(); zoom(-0.1); }
      else if (ctrl && key === '0') { handled(); zoom(0); }
      else if (ctrl && (key === 'pagedown' || (key === 'tab' && !event.shiftKey)) || ctrl && key === 'pageup' || ctrl && event.shiftKey && key === 'tab') {
        handled();
        const list = tabsRef.current; const index = list.findIndex(t => t.id === id);
        const step = key === 'pageup' || event.shiftKey ? -1 : 1;
        if (list.length) setActive(list[(index + step + list.length) % list.length].id);
      } else if (event.altKey && /^[1-9]$/.test(event.key)) { handled(); const tab = tabsRef.current[Number(event.key) - 1]; if (tab) setActive(tab.id); }
      else if (event.key === 'Escape' && (menu || panel)) { handled(); setMenu(null); setPanel(null); }
    };
    window.addEventListener('keydown', keydown, true);
    return () => window.removeEventListener('keydown', keydown, true);
  });

  const tab = tabOf(active);
  const titleOf = (t: Tab) => `${t.title}${dirty[t.id] ? ' •' : ''}`;
  useEffect(() => { document.title = tab ? `${titleOf(tab)} — Athena` : 'Athena'; });
  const font = settings.font === 'mono' ? 'var(--ayla-font-mono)' : 'var(--ayla-font-main)';
  const languageLabel = tab ? (tab.language ?? languageFor(tab.path ?? tab.note?.path ?? tab.title)?.name ?? 'Texto simples') : '';
  const indentLabel = settings.spaces ? `Espaços: ${settings.tabWidth}` : `Largura do tab: ${settings.tabWidth}`;
  const set = (change: Partial<Settings>) => setSettings(s => ({ ...s, ...change }));
  const setTabOption = async (change: Partial<Tab>) => {
    if (!tab) return; const updated = { ...tab, ...change }; patchTab(tab.id, change);
    if ('language' in change) { await loadLanguage(updated); applyConfig(tab.id); }
  };

  return <div className="pad" style={{ ['--pad-font' as string]: font, ['--pad-size' as string]: `${settings.fontSize * settings.zoom}px` }}>
    <header className="pad-bar" data-tauri-drag-region>
      <span className="pad-brand" title="Athena" data-tauri-drag-region><BrandMark name="athena" size={24} variant={look.icon_athena} /></span>
      <button className="pad-button pad-open" aria-haspopup="menu" aria-expanded={menu === 'open'} onClick={() => setMenu(m => m === 'open' ? null : 'open')}>Abrir<ChevronDown /></button>
      <button className="pad-icon" aria-label="Nova aba" title="Nova aba (Ctrl+T)" onClick={newDocument}><NewTabIcon /></button>
      <div className="pad-title" data-tauri-drag-region>{tab && <><b>{titleOf(tab)}</b><small>{tab.note ? 'Nota da Athena' : homeShort(folderOf(tab.path)) ?? 'Rascunho'}</small></>}</div>
      <button className="pad-icon" aria-label="Propriedades do documento" title="Propriedades do documento" aria-haspopup="menu" aria-expanded={menu === 'info'} disabled={!tab} onClick={() => setMenu(m => m === 'info' ? null : 'info')}><InfoIcon /></button>
      <button className="pad-icon" aria-label="Menu principal" title="Menu principal" aria-haspopup="menu" aria-expanded={menu === 'main'} onClick={() => setMenu(m => m === 'main' ? null : 'main')}><MenuIcon /></button>
      <button className="pad-window-close" aria-label="Fechar a janela" title="Fechar" onClick={() => void getCurrentWindow().close()}><CloseIcon size={14} /></button>
    </header>
    <nav className="pad-tabs" aria-label="Documentos abertos" ref={tabsNav} hidden={tabs.length < 2}>
      {tabs.map((t, i) => <div key={t.id} data-id={t.id} className={`pad-tab${dragging === t.id ? ' dragging' : ''}`} aria-current={t.id === active ? 'page' : undefined} title={t.path ?? (t.note ? `Nota da Athena: ${t.note.path}` : t.title)}
        onMouseDown={e => { if (e.button === 1) { e.preventDefault(); requestClose(t.id); } }}
        onPointerDown={e => dragStart(e, t.id)} onPointerMove={e => dragMove(e, t.id)} onPointerUp={dragEnd} onPointerCancel={dragEnd} onLostPointerCapture={dragEnd}>
        <button className="pad-tab-name" onClick={() => setActive(t.id)}>{t.note && <FileIcon size={13} />}<span>{titleOf(t)}</span>{i < 9 && <kbd>Alt+{i + 1}</kbd>}</button>
        <button className="pad-tab-close" aria-label={`Fechar ${t.title}`} onClick={() => requestClose(t.id)}><CloseIcon size={13} /></button>
      </div>)}
    </nav>

    {menu === 'open' && <div className="pad-menu pad-menu-left" role="menu">
      <button role="menuitem" onClick={() => void openDialog()}>Escolher no sistema…<kbd>Ctrl+O</kbd></button>
      <button role="menuitem" onClick={() => { setMenu(null); setPanel('notes'); }}>Notas da Athena…</button>
      <hr />
      <p className="pad-menu-head">Recentes</p>
      {!recent.length && <p className="pad-hint">Nenhum arquivo recente.</p>}
      {recent.map(path => <button key={path} role="menuitem" className="pad-recent" title={path} onClick={() => { setMenu(null); void openPath(path); }}><b>{path.split('/').pop()}</b><small>{folderOf(path)}</small></button>)}
    </div>}
    {menu === 'main' && tab && <div className="pad-menu" role="menu">
      <button role="menuitem" onClick={() => { setMenu(null); newDocument(); }}>Nova aba<kbd>Ctrl+T</kbd></button>
      <hr />
      <button role="menuitem" onClick={() => { setMenu(null); void save(tab.id); }}>Salvar<kbd>Ctrl+S</kbd></button>
      <button role="menuitem" onClick={() => { setMenu(null); void saveAs(tab.id); }}>Salvar como…<kbd>Ctrl+Shift+S</kbd></button>
      <button role="menuitem" disabled={!tab.path && !tab.note} onClick={() => { setMenu(null); void reload(tab.id); }}>Descartar alterações</button>
      <button role="menuitem" onClick={print}>Imprimir…<kbd>Ctrl+P</kbd></button>
      <hr />
      <button role="menuitem" onClick={() => { setMenu(null); if (view.current) openSearchPanel(view.current); }}>Localizar e substituir<kbd>Ctrl+F</kbd></button>
      <button role="menuitem" onClick={() => { setMenu(null); if (view.current) gotoLine(view.current); }}>Ir para a linha<kbd>Ctrl+I</kbd></button>
      <hr />
      <div className="pad-zoom" role="group" aria-label="Zoom">
        <button aria-label="Diminuir o zoom (Ctrl+-)" title="Diminuir o zoom (Ctrl+-)" onClick={() => zoom(-0.1)}>−</button>
        <button title="Zoom normal (Ctrl+0)" onClick={() => zoom(0)}>{Math.round(settings.zoom * 100)}%</button>
        <button aria-label="Aumentar o zoom (Ctrl++)" title="Aumentar o zoom (Ctrl++)" onClick={() => zoom(0.1)}>+</button>
      </div>
      <hr />
      <p className="pad-menu-head">Texto</p>
      <button role="menuitem" onClick={() => caseChange('upper')}>Maiúsculas</button>
      <button role="menuitem" onClick={() => caseChange('lower')}>Minúsculas</button>
      <button role="menuitem" onClick={() => caseChange('invert')}>Inverter maiúsculas e minúsculas</button>
      <button role="menuitem" onClick={() => caseChange('title')}>Primeira letra maiúscula</button>
      <button role="menuitem" onClick={() => transform(t => sortLines(t))}>Ordenar linhas</button>
      <button role="menuitem" onClick={() => transform(t => sortLines(t, { unique: true }))}>Ordenar e remover repetidas</button>
      <button role="menuitem" onClick={() => transform(joinLines)}>Juntar linhas</button>
      <hr />
      <button role="menuitem" onClick={() => { setMenu(null); setPanel('prefs'); }}>Preferências</button>
      <button role="menuitem" onClick={() => { setMenu(null); requestClose(tab.id); }}>Fechar aba<kbd>Ctrl+W</kbd></button>
    </div>}
    {menu === 'info' && tab && <div className="pad-menu pad-info" role="menu">
      <p className="pad-info-pos">Ln {cursor.line}, Col {cursor.col}{cursor.selected ? ` · ${cursor.selected} selecionados` : ''}</p>
      <label>Linguagem<select value={languageLabel} onChange={e => void setTabOption({ language: e.target.value })}>{languageNames.map(name => <option key={name}>{name}</option>)}</select></label>
      <label>Recuo<select value={`${settings.spaces ? 's' : 't'}${settings.tabWidth}`} onChange={e => set({ spaces: e.target.value.startsWith('s'), tabWidth: Number(e.target.value.slice(1)) })} title={indentLabel}>
        {[2, 4, 8].map(n => <option key={`t${n}`} value={`t${n}`}>Tab ({n})</option>)}{[2, 4, 8].map(n => <option key={`s${n}`} value={`s${n}`}>Espaços ({n})</option>)}
      </select></label>
      <label>Fim de linha<select value={tab.eol} disabled={Boolean(tab.note)} onChange={e => void setTabOption({ eol: e.target.value as Tab['eol'] }).then(() => setRevision(r => r + 1))}><option value={'\n'}>Unix (LF)</option><option value={'\r\n'}>Windows (CRLF)</option></select></label>
      <label>Codificação<select value={tab.encoding} disabled={Boolean(tab.note)} onChange={e => void setTabOption({ encoding: e.target.value })}><option>UTF-8</option><option>ISO-8859-1</option></select></label>
    </div>}

    {error && <div className="pad-banner error" role="alert"><span>{error}</span><button onClick={() => setError('')}>Fechar</button></div>}
    {notice && <div className="pad-banner" role="status"><span>{notice}</span><button onClick={() => setNotice('')}>Fechar</button></div>}
    {tab?.missing && tab.path && <div className="pad-banner warn" role="alert"><span>O arquivo não existe mais no disco. Salvar cria de novo.</span></div>}
    {tab?.readonly && <div className="pad-banner" role="status"><span>Somente leitura: sem permissão para gravar este arquivo. Use “Salvar como”.</span></div>}
    {tab && (tab.changedOnDisk || conflict === tab.id) && <div className="pad-banner warn" role="alert">
      <span>O arquivo mudou no disco depois que foi aberto.</span>
      <button onClick={() => void reload(tab.id)}>Recarregar</button>
      <button onClick={() => void save(tab.id, true)}>Manter as minhas alterações e salvar</button>
    </div>}
    {closing && tabOf(closing.id) && <div className="pad-banner warn" role="alertdialog" aria-label="Fechar com alterações">
      <span>{closing.discard ? `Descartar mesmo as alterações em “${tabOf(closing.id)!.title}”? Isso não pode ser desfeito.` : `Salvar as alterações em “${tabOf(closing.id)!.title}” antes de fechar?`}</span>
      {closing.discard
        ? <><button className="danger" onClick={() => closeTab(closing.id)}>Descartar</button><button onClick={() => setClosing(null)}>Cancelar</button></>
        : <><button className="primary" onClick={() => void save(closing.id).then(ok => { if (ok) closeTab(closing.id); })}>Salvar</button>
            <button onClick={() => setClosing({ ...closing, discard: true })}>Não salvar</button><button onClick={() => setClosing(null)}>Cancelar</button></>}
    </div>}

    <main className="pad-editor" ref={host} onClick={() => setMenu(null)} />

    {panel === 'prefs' && <Preferences settings={settings} set={set} onClose={() => setPanel(null)} />}
    {panel === 'notes' && <NotesPicker onPick={note => void openNote(note)} onClose={() => setPanel(null)} />}
    {printing !== null && <pre className="pad-print">{printing}</pre>}
  </div>;
}

function Toggle({ label, value, change }: { label: string; value: boolean; change: (v: boolean) => void }) {
  return <label className="pad-toggle"><span>{label}</span><input type="checkbox" role="switch" checked={value} onChange={e => change(e.target.checked)} /></label>;
}

/** Preferências do editor (as mesmas do Text Editor). */
function Preferences({ settings, set, onClose }: { settings: Settings; set: (c: Partial<Settings>) => void; onClose: () => void }) {
  return <div className="pad-dialog-backdrop" onClick={onClose}>
    <div className="pad-dialog" role="dialog" aria-modal="true" aria-label="Preferências" onClick={e => e.stopPropagation()}>
      <div className="pad-dialog-head"><h2>Preferências</h2><button className="pad-icon" aria-label="Fechar" onClick={onClose}><CloseIcon size={16} /></button></div>
      <h3>Aparência</h3>
      <label className="pad-field">Fonte<select value={settings.font} onChange={e => set({ font: e.target.value as Settings['font'] })}><option value="mono">Monoespaçada</option><option value="system">Fonte da Ayla</option></select></label>
      <label className="pad-field">Tamanho<input type="number" min={8} max={40} value={settings.fontSize} onChange={e => set({ fontSize: Math.min(40, Math.max(8, Number(e.target.value) || 15)) })} /></label>
      <Toggle label="Mostrar números das linhas" value={settings.lineNumbers} change={v => set({ lineNumbers: v })} />
      <Toggle label="Destacar a linha atual" value={settings.highlightLine} change={v => set({ highlightLine: v })} />
      <Toggle label="Mostrar a margem direita" value={settings.rightMargin} change={v => set({ rightMargin: v })} />
      {settings.rightMargin && <label className="pad-field">Coluna da margem<input type="number" min={20} max={300} value={settings.marginColumn} onChange={e => set({ marginColumn: Math.min(300, Math.max(20, Number(e.target.value) || 80)) })} /></label>}
      <Toggle label="Mostrar espaços e tabs" value={settings.showSpaces} change={v => set({ showSpaces: v })} />
      <h3>Comportamento</h3>
      <Toggle label="Quebrar o texto" value={settings.wrap} change={v => set({ wrap: v })} />
      <Toggle label="Verificar a ortografia" value={settings.spellcheck} change={v => set({ spellcheck: v })} />
      <Toggle label="Realçar parênteses e colchetes correspondentes" value={settings.brackets} change={v => set({ brackets: v })} />
      <Toggle label="Restaurar a sessão ao abrir (abas e rascunhos)" value={settings.restoreSession} change={v => set({ restoreSession: v })} />
      <h3>Recuo</h3>
      <label className="pad-field">Recuar com<select value={settings.spaces ? 'spaces' : 'tabs'} onChange={e => set({ spaces: e.target.value === 'spaces' })}><option value="tabs">Tabs</option><option value="spaces">Espaços</option></select></label>
      <label className="pad-field">Largura<input type="number" min={1} max={16} value={settings.tabWidth} onChange={e => set({ tabWidth: Math.min(16, Math.max(1, Number(e.target.value) || 8)) })} /></label>
      <Toggle label="Recuo automático" value={settings.autoIndent} change={v => set({ autoIndent: v })} />
    </div>
  </div>;
}

/** Notas da Athena (e dos projetos da Ayla), sem os cofres externos, com busca pelo título ou caminho. */
function NotesPicker({ onPick, onClose }: { onPick: (note: NoteNode) => void; onClose: () => void }) {
  const [notes, setNotes] = useState<NoteNode[] | null>(null); const [query, setQuery] = useState(''); const [error, setError] = useState('');
  useEffect(() => { void native<{ graph: { nodes: NoteNode[] } }>('athena_overview').then(o => setNotes(o.graph.nodes.filter(n => !n.root.startsWith('vault:')).sort((a, b) => a.title.localeCompare(b.title, 'pt-BR')))).catch(e => setError(errorText(e))); }, []);
  const term = query.trim().toLocaleLowerCase('pt-BR');
  const shown = (notes ?? []).filter(n => !term || `${n.title} ${n.path}`.toLocaleLowerCase('pt-BR').includes(term)).slice(0, 200);
  return <div className="pad-dialog-backdrop" onClick={onClose}>
    <div className="pad-dialog pad-dialog-notes" role="dialog" aria-modal="true" aria-label="Notas da Athena" onClick={e => e.stopPropagation()}>
      <div className="pad-dialog-head"><h2>Notas da Athena</h2><button className="pad-icon" aria-label="Fechar" onClick={onClose}><CloseIcon size={16} /></button></div>
      <input className="pad-search" autoFocus placeholder="Buscar nota" value={query} onChange={e => setQuery(e.target.value)} onKeyDown={e => { if (e.key === 'Enter' && shown[0]) onPick(shown[0]); }} />
      {error && <p className="pad-hint">{error}</p>}
      {notes === null && !error && <p className="pad-hint">Carregando…</p>}
      {notes && !shown.length && <p className="pad-hint">Nenhuma nota encontrada.</p>}
      <ul className="pad-notes">{shown.map(n => <li key={`${n.root}|${n.path}`}><button onClick={() => onPick(n)}><b>{n.title}</b><small>{n.rootName} · {n.path}</small></button></li>)}</ul>
    </div>
  </div>;
}

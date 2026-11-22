import { useCallback, useEffect, useLayoutEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import Chat from './Chat';
import MemoryPanel from './MemoryPanel';
import AppVersion from './AppVersion';
import { native, errorText, fileLabels, formatBytes, type Conversation, type FileInfo, type Project, type Snapshot } from '../services/Desktop';
import { ChevronLeftIcon, ChevronRightIcon, CloudIcon, ComputerIcon, FileIcon, PencilIcon, PlusIcon, SidebarIcon, TrashIcon } from './Icons';

type View = 'chat' | 'overview' | 'memory' | 'files';
type Slot = { key: string; conversation: Conversation | null; busy: boolean };
let slotCounter = 0;
type Props = {
  snapshot: Snapshot; refresh: () => Promise<void>;
  /** `null` é o espaço geral da Ayla; senão, um projeto. */
  project: Project | null;
  /** Espaço visível no momento. */
  active: boolean;
  compact?: boolean;
  quickOpening?: number | null;
  onDeleted?: () => void;
  onOpenSettings?: (section: string) => void;
  /** Na barra da Ayla: lista de projetos, abrir e criar. */
  projects?: Project[];
  onOpenProject?: (id: string) => void;
  onNewProject?: () => void;
  /** No projeto: voltar para a conversa geral da Ayla. */
  onBack?: () => void;
};

/** Iniciais e cor estável de um projeto, usadas no trilho e no cabeçalho. */
export function ProjectAvatar({ project, size = 36 }: { project: Project; size?: number }) {
  const palette = ['#7d9bff', '#4fc79b', '#e8ad5a', '#e07fb0', '#a294f5', '#5cc2d6'];
  const hash = [...project.id].reduce((sum, c) => sum + c.charCodeAt(0), 0);
  const initials = project.name.trim().split(/\s+/).slice(0, 2).map(word => word[0]).join('').toUpperCase() || '?';
  return <span className="avatar" style={{ width: size, height: size, background: palette[hash % palette.length] }} aria-hidden="true">{initials}</span>;
}

/** Um espaço: a conversa geral da Ayla ou um projeto, com barra própria. */
export default function Space({ snapshot, refresh, project, active, compact = false, quickOpening, onDeleted, onOpenSettings, projects = [], onOpenProject, onNewProject, onBack }: Props) {
  const [conversations, setConversations] = useState<Conversation[]>([]);
  // Cada conversa respondendo fica montada (oculta) para a resposta seguir enquanto outra é aberta.
  const [slots, setSlots] = useState<Slot[]>([{ key: 'c0', conversation: null, busy: false }]);
  const [activeKey, setActiveKey] = useState('c0');
  const conversation = slots.find(s => s.key === activeKey)?.conversation ?? null;
  /** Troca a conversa em todas as abas onde ela aparece (título, permissão). */
  const replaceConversation = useCallback((id: string, change: (c: Conversation) => Conversation | null) => {
    setSlots(prev => prev.map(s => s.conversation?.id === id ? { ...s, conversation: change(s.conversation) } : s));
  }, []);
  const [view, setView] = useState<View>(project ? 'overview' : 'chat');
  const [narrow, setNarrow] = useState(() => window.matchMedia('(max-width: 1000px)').matches);
  const [normalSidebar, setNormalSidebar] = useState(true);
  const [overlaySidebar, setOverlaySidebar] = useState(false);
  useEffect(() => {
    const query = window.matchMedia('(max-width: 1000px)');
    const update = () => { setNarrow(query.matches); setOverlaySidebar(false); };
    query.addEventListener('change', update);
    return () => query.removeEventListener('change', update);
  }, []);
  const [compactSidebar, setCompactSidebar] = useState(false);
  const sidebar = compact ? compactSidebar : narrow ? overlaySidebar : normalSidebar;
  const setSidebar = compact ? setCompactSidebar : narrow ? setOverlaySidebar : setNormalSidebar;
  const focusComposer = compact && active ? quickOpening : null;
  useLayoutEffect(() => {
    if (focusComposer == null) return;
    setView('chat');
    setCompactSidebar(false);
  }, [focusComposer]);
  const [error, setError] = useState('');
  const [editing, setEditing] = useState<string | null>(null); const [draftTitle, setDraftTitle] = useState(''); const [confirming, setConfirming] = useState<string | null>(null);
  const projectId = project?.id ?? null;
  const mode = snapshot.preferences.usage_mode;
  const where = mode === 'local_only' ? 'Tudo roda neste computador' : mode === 'cloud_only' ? 'Respostas pela nuvem, com sua revisão' : 'Nuvem disponível';

  useEffect(() => { native<Conversation[]>('conversations', { project: projectId }).then(setConversations).catch(e => setError(errorText(e))); }, [projectId]);
  // Título resumido pela Ayla depois da primeira resposta.
  useEffect(() => {
    const stop = listen<{ id: string; title: string }>('conversation-updated', event => {
      const { id, title } = event.payload;
      setConversations(prev => prev.map(c => c.id === id ? { ...c, title } : c));
      replaceConversation(id, current => ({ ...current, title }));
    });
    return () => { void stop.then(unlisten => unlisten()); };
  }, [replaceConversation]);
  const onConversation = useCallback((key: string, item: Conversation) => {
    setSlots(prev => prev.map(s => s.key === key ? { ...s, conversation: item } : s));
    setConversations(prev => prev.some(c => c.id === item.id) ? prev.map(c => c.id === item.id ? item : c) : [item, ...prev]);
  }, []);
  /** Uma conversa que terminou de responder fora da tela deixa de ficar montada. */
  const setSlotBusy = useCallback((key: string, busy: boolean) => {
    setSlots(prev => {
      const slot = prev.find(s => s.key === key);
      if (!slot || slot.busy === busy) return prev;
      return prev.map(s => s.key === key ? { ...s, busy } : s);
    });
  }, []);
  useEffect(() => { setSlots(prev => prev.some(s => s.key !== activeKey && !s.busy) ? prev.filter(s => s.key === activeKey || s.busy) : prev); }, [slots, activeKey]);
  const busyIds = new Set(slots.filter(s => s.busy && s.conversation).map(s => s.conversation!.id));
  /** Atualiza uma conversa na lista (e a aberta) sem trocar de conversa. */
  const patch = useCallback((item: Conversation) => {
    setConversations(prev => prev.map(c => c.id === item.id ? item : c));
    replaceConversation(item.id, () => item);
  }, [replaceConversation]);
  async function rename(item: Conversation) {
    const title = draftTitle.trim(); setEditing(null);
    if (!title || title === item.title) return;
    try { await native('rename_conversation', { id: item.id, title }); patch({ ...item, title }); } catch (e) { setError(errorText(e)); }
  }
  async function remove(item: Conversation) {
    setConfirming(null);
    try {
      await native('delete_conversation', { id: item.id });
      setConversations(prev => prev.filter(c => c.id !== item.id));
      replaceConversation(item.id, () => null);
      await refresh();
    } catch (e) { setError(errorText(e)); }
  }
  function openConversation(item: Conversation | null) {
    const open = item && slots.find(s => s.conversation?.id === item.id);
    const current = slots.find(s => s.key === activeKey);
    if (open) setActiveKey(open.key);
    else if (current && !current.busy) setSlots(prev => prev.map(s => s.key === activeKey ? { ...s, conversation: item } : s));
    else {
      const key = `c${++slotCounter}`;
      setSlots(prev => [...prev, { key, conversation: item, busy: false }]); setActiveKey(key);
    }
    setView('chat');
    if (compact || narrow) setSidebar(false);
  }

  return <div className="chat-layout">
    {(compact || narrow) && sidebar && <div className="sidebar-backdrop" aria-hidden="true" onClick={() => setSidebar(false)} />}
    {sidebar && <aside className="sidebar" aria-label={project ? `Projeto ${project.name}` : 'Conversas'}>
      {project && onBack && <button className="ghost-button back-to-ayla" onClick={onBack}><ChevronLeftIcon size={16} />Ayla</button>}
      <div className="brand">
        {project ? <ProjectAvatar project={project} size={28} /> : <span className="orb" aria-hidden="true" />}
        <span className="brand-name" title={project?.name}>{project ? project.name : 'Ayla'}</span>
        <button className="icon-button" aria-label="Recolher a barra lateral" onClick={() => setSidebar(false)}><SidebarIcon /></button>
      </div>
      {project && <nav className="space-nav" aria-label="Seções do projeto">
        <button className="conversation-item" aria-current={view === 'overview' ? 'page' : undefined} onClick={() => setView('overview')}>Visão geral</button>
        <button className="conversation-item" aria-current={view === 'memory' ? 'page' : undefined} onClick={() => setView('memory')}>Memória</button>
        <button className="conversation-item" aria-current={view === 'files' ? 'page' : undefined} onClick={() => setView('files')}>Arquivos</button>
      </nav>}
      <button className="new-chat" onClick={() => openConversation(null)}><PlusIcon size={18} />Nova conversa</button>
      <nav className="conversation-list" aria-label="Conversas e projetos">
        {!project && <>
          <div className="sidebar-label-row"><p className="sidebar-label">Projetos</p><button className="icon-button small" aria-label="Novo projeto" title="Novo projeto" onClick={onNewProject}><PlusIcon size={16} /></button></div>
          {!projects.length && <button className="conversation-item muted" onClick={onNewProject}>Criar um projeto…</button>}
          {projects.map(item => <button key={item.id} className="conversation-item project-item" onClick={() => onOpenProject?.(item.id)}><ProjectAvatar project={item} size={22} /><span>{item.name}</span></button>)}
        </>}
        {conversations.length > 0 && <p className="sidebar-label">Conversas</p>}
        {conversations.map(item => editing === item.id
          ? <form key={item.id} className="conversation-edit" onSubmit={e => { e.preventDefault(); void rename(item); }}>
              <input autoFocus aria-label="Título da conversa" value={draftTitle} maxLength={120} onChange={e => setDraftTitle(e.target.value)} onBlur={() => void rename(item)} onKeyDown={e => { if (e.key === 'Escape') { e.preventDefault(); setEditing(null); } }} />
            </form>
          : confirming === item.id
          ? <div key={item.id} className="conversation-confirm" role="group" aria-label="Confirmar exclusão">
              <span>Apagar “{item.title}”?</span>
              <small>Mensagens e arquivos dela somem. O que a Ayla aprendeu fica em Memória.</small>
              <div className="row-actions"><button className="danger-action" onClick={() => void remove(item)}>Apagar</button><button className="ghost-button" onClick={() => setConfirming(null)}>Cancelar</button></div>
            </div>
          : <div key={item.id} className="conversation-row">
              <button className="conversation-item" aria-current={view === 'chat' && item.id === conversation?.id ? 'page' : undefined} onClick={() => openConversation(item)} onDoubleClick={() => { setEditing(item.id); setDraftTitle(item.title); }} title={item.title}>{item.title}</button>
              <span className="row-tools">
                <button className="icon-button tiny" aria-label={`Renomear “${item.title}”`} title="Renomear" onClick={() => { setConfirming(null); setEditing(item.id); setDraftTitle(item.title); }}><PencilIcon size={15} /></button>
                <button className="icon-button tiny" aria-label={`Apagar “${item.title}”`} title="Apagar" disabled={busyIds.has(item.id)} onClick={() => { setEditing(null); setConfirming(item.id); }}><TrashIcon size={15} /></button>
              </span>
            </div>)}
        {error && view === 'chat' && <p className="sidebar-error" role="alert">{error}</p>}
      </nav>
      {!project && <div className="sidebar-footer"><p className="where">{mode === 'cloud_only' ? <CloudIcon size={18} /> : <ComputerIcon size={18} />}{where}</p><AppVersion /></div>}
    </aside>}

    {/* A conversa fica montada ao ver a visão geral ou a memória: uma resposta em andamento continua. */}
    {slots.map(slot => <div key={slot.key} className="chat-slot" hidden={view !== 'chat' || slot.key !== activeKey}>
      <Chat snapshot={snapshot} refresh={refresh} project={project} conversation={slot.conversation} onConversation={item => onConversation(slot.key, item)} onBusy={busy => setSlotBusy(slot.key, busy)} onShowSidebar={sidebar ? undefined : () => setSidebar(true)} active={active && view === 'chat' && slot.key === activeKey} focusComposer={slot.key === activeKey ? focusComposer : null} onOpenSettings={onOpenSettings} />
    </div>)}
    {project && view === 'overview' && <ProjectOverview project={project} conversations={conversations} refresh={refresh} onOpen={openConversation} onDeleted={onDeleted} onShowSidebar={sidebar ? undefined : () => setSidebar(true)} error={error} />}
    {project && view === 'memory' && <main className="settings-main"><div className="settings-content">
      {!sidebar && <button className="ghost-button back-button-inline" onClick={() => setSidebar(true)}><SidebarIcon size={18} />Mostrar a barra lateral</button>}
      <MemoryPanel snapshot={snapshot} refresh={refresh} project={project} />
    </div></main>}
    {project && view === 'files' && <ProjectFiles project={project} onShowSidebar={sidebar ? undefined : () => setSidebar(true)} />}
  </div>;
}

type OverviewProps = {
  project: Project; conversations: Conversation[]; refresh: () => Promise<void>; error: string;
  onOpen: (conversation: Conversation | null) => void; onDeleted?: () => void; onShowSidebar?: () => void;
};
function ProjectOverview({ project, conversations, refresh, onOpen, onDeleted, onShowSidebar, error: loadError }: OverviewProps) {
  const [editing, setEditing] = useState(false); const [name, setName] = useState(project.name); const [description, setDescription] = useState(project.description);
  const [confirmDelete, setConfirmDelete] = useState(false); const [saving, setSaving] = useState(false); const [error, setError] = useState('');
  async function save() {
    setSaving(true); setError('');
    try { await native('update_project', { id: project.id, name, description }); await refresh(); setEditing(false); }
    catch (e) { setError(errorText(e)); } finally { setSaving(false); }
  }
  async function remove() {
    setSaving(true); setError('');
    try { await native('delete_project', { id: project.id }); await refresh(); onDeleted?.(); }
    catch (e) { setError(errorText(e)); setSaving(false); }
  }
  return <main className="settings-main"><div className="settings-content">
    {onShowSidebar && <button className="ghost-button back-button-inline" onClick={onShowSidebar}><SidebarIcon size={18} />Mostrar a barra lateral</button>}
    <header className="project-header">
      <ProjectAvatar project={project} size={48} />
      <div className="row-text"><h1>{project.name}</h1><p className="row-hint">{project.conversations} {project.conversations === 1 ? 'conversa' : 'conversas'} · {project.memories} {project.memories === 1 ? 'memória' : 'memórias'}</p></div>
      <button className="primary-action with-icon" onClick={() => onOpen(null)}><PlusIcon size={18} />Nova conversa</button>
    </header>
    {(error || loadError) && <p role="alert" className="app-error">{error || loadError}</p>}

    <section className="settings-group" aria-labelledby="about-title"><h2 id="about-title">Sobre o projeto</h2>
      <div className="group-card">
        {editing ? <div className="group-row stacked">
          <label className="row-title" htmlFor="project-name">Nome</label>
          <input id="project-name" value={name} maxLength={80} onChange={e => setName(e.target.value)} />
          <label className="row-title" htmlFor="project-about">Sobre o projeto</label>
          <span className="row-hint">A Ayla usa isso para entender o foco das conversas deste projeto (só nas respostas neste computador).</span>
          <textarea id="project-about" rows={4} maxLength={2000} value={description} onChange={e => setDescription(e.target.value)} placeholder="Ex.: TCC sobre energia solar no Nordeste; entrega em junho." />
          <div className="row-actions"><button className="primary-action" disabled={saving || !name.trim()} onClick={() => void save()}>Salvar</button><button className="ghost-button" onClick={() => { setEditing(false); setName(project.name); setDescription(project.description); }}>Cancelar</button></div>
        </div> : <div className="group-row">
          <span className="row-text"><span className={project.description ? 'about-text' : 'row-hint'}>{project.description || 'Conte à Ayla do que se trata este projeto para ela manter o foco.'}</span></span>
          <button onClick={() => { setName(project.name); setDescription(project.description); setEditing(true); }}>Editar</button>
        </div>}
      </div>
    </section>

    <section className="settings-group" aria-labelledby="recent-title"><h2 id="recent-title">Conversas recentes</h2>
      <div className="group-card">
        {!conversations.length && <div className="group-row"><span className="row-hint">Nenhuma conversa ainda. Comece uma nova; ela fica guardada neste projeto.</span></div>}
        {conversations.slice(0, 6).map(item => <button key={item.id} className="group-row link-row" onClick={() => onOpen(item)}><span className="row-text"><span className="row-title">{item.title}</span></span><ChevronRightIcon size={18} /></button>)}
      </div>
    </section>

    <section className="settings-group" aria-labelledby="danger-title"><h2 id="danger-title">Apagar projeto</h2>
      <div className="group-card">
        {confirmDelete ? <div className="group-row stacked" role="group" aria-label="Confirmar exclusão">
          <span>Apagar <b>{project.name}</b> com {project.conversations} {project.conversations === 1 ? 'conversa' : 'conversas'} e {project.memories} {project.memories === 1 ? 'memória' : 'memórias'}? Isso não pode ser desfeito; o backup em Configurações guarda uma cópia antes. As notas do projeto vão para a lixeira da Athena (Athena › .trash).</span>
          <div className="row-actions"><button className="danger-action" disabled={saving} onClick={() => void remove()}>Apagar de vez</button><button className="ghost-button" onClick={() => setConfirmDelete(false)}>Cancelar</button></div>
        </div> : <div className="group-row">
          <span className="row-text"><span className="row-hint">Remove o projeto, as conversas, as memórias e os arquivos dele; as notas vão para a lixeira da Athena. As memórias gerais não são afetadas.</span></span>
          <button onClick={() => setConfirmDelete(true)}>Apagar…</button>
        </div>}
      </div>
    </section>
  </div></main>;
}

/** Arquivos guardados no projeto: enviados nas conversas ou adicionados aqui. */
function ProjectFiles({ project, onShowSidebar }: { project: Project; onShowSidebar?: () => void }) {
  const [items, setItems] = useState<FileInfo[] | null>(null); const [error, setError] = useState('');
  const [busy, setBusy] = useState(false); const [confirm, setConfirm] = useState<string | null>(null);
  const load = useCallback(() => native<FileInfo[]>('project_files', { project: project.id }).then(setItems).catch(e => setError(errorText(e))), [project.id]);
  useEffect(() => { void load(); }, [load]);
  async function add() {
    setError('');
    try {
      const paths = await native<string[]>('pick_files');
      setBusy(true);
      for (const path of paths) await native('add_file', { path, project: project.id, keep: true });
    } catch (e) { setError(errorText(e)); } finally { setBusy(false); await load(); }
  }
  async function remove(id: string) {
    setConfirm(null); setError('');
    try { await native('remove_file', { id }); } catch (e) { setError(errorText(e)); } finally { await load(); }
  }
  return <main className="settings-main"><div className="settings-content">
    {onShowSidebar && <button className="ghost-button back-button-inline" onClick={onShowSidebar}><SidebarIcon size={18} />Mostrar a barra lateral</button>}
    <header className="project-header">
      <div className="row-text"><h1>Arquivos</h1><p className="row-hint">Cópias guardadas pela Ayla neste computador. Entram no backup e nunca vão para a nuvem.</p></div>
      <button className="primary-action with-icon" disabled={busy} onClick={() => void add()}><PlusIcon size={18} />{busy ? 'Guardando…' : 'Adicionar arquivos'}</button>
    </header>
    {error && <p role="alert" className="app-error">{error}</p>}
    <div className="group-card">
      {items && !items.length && <div className="group-row"><span className="row-hint">Nenhum arquivo ainda. Anexe nas conversas do projeto ou adicione aqui: imagens, GIFs, PDFs, textos, áudios e vídeos.</span></div>}
      {items?.map(file => <div key={file.id} className="group-row">
        <FileIcon size={20} />
        <span className="row-text"><span className="row-title file-title">{file.name}</span><span className="row-hint">{fileLabels[file.kind]} · {formatBytes(file.bytes)} · {new Date(file.created_at * 1000).toLocaleDateString('pt-BR')}{file.message_id ? ' · enviado numa conversa' : ''}</span></span>
        {confirm === file.id
          ? <span className="row-actions"><button className="danger-action" onClick={() => void remove(file.id)}>Apagar</button><button className="ghost-button" onClick={() => setConfirm(null)}>Cancelar</button></span>
          : <button onClick={() => setConfirm(file.id)}>Apagar…</button>}
      </div>)}
    </div>
  </div></main>;
}

import { Fragment, useCallback, useEffect, useLayoutEffect, useRef, useState, type ClipboardEvent, type FormEvent } from 'react';
import Confirm from './Confirm';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { native, nativeTask, taskId, errorText, formatBytes, supports, savePreferences, fileLabels, type Capability, type Conversation, type Fact, type FileInfo, type Message, type Project, type Sensitivity, type Snapshot, type ChatRequest, type DesignCard, type Effort, type Reference, type OfferedNote, type KeptNote, type CreatedNote } from '../services/Desktop';
import { base64, recordWav, RECORD_LIMIT_MS, type Recording } from '../services/Audio';
import { asksForImage, asksAboutScreen } from '../services/Intent';
import ImageResults from './ImageResults';
import ChatDesigns from './ChatDesigns';
import ChatFicha from './ChatFicha';
import ChatHand, { type HandCard } from './ChatHand';
import type { FichaCard } from '../services/Ficha';
import BrandMark from './BrandMark';
import { ArrowUpIcon, CloseIcon, CopyIcon, DatabaseIcon, FileIcon, ImageIcon, LayoutIcon, MicIcon, ChevronLeftIcon, ChevronRightIcon, PaperclipIcon, PencilIcon, RefreshIcon, ScreenIcon, SidebarIcon, SlidersIcon, StopIcon } from './Icons';
import Markdown from './Markdown';
import FilePreview, { AttachmentThumb } from './FilePreview';
import './Chat.css';

type Props = {
  snapshot: Snapshot; refresh: () => Promise<void>;
  /** Projeto do espaço; `null` é a conversa geral da Ayla. */
  project: Project | null;
  /** Conversa aberta; `null` mostra uma conversa nova, criada na primeira mensagem. */
  conversation: Conversation | null;
  /** Conversa criada ou alterada aqui (primeira mensagem, consentimento). */
  onConversation: (conversation: Conversation) => void;
  onBusy: (busy: boolean) => void;
  /** Botão para mostrar a barra do espaço quando ela estiver recolhida. */
  onShowSidebar?: () => void;
  /** Conversa visível: só ela recebe arquivos arrastados para a janela. */
  active: boolean;
  focusComposer?: number | null;
  /** Abre uma seção das Configurações (ex.: "nuvem"). */
  onOpenSettings?: (section: string) => void;
  /** Abre uma nota (caminho do arquivo) na Athena. */
  onOpenNote?: (path: string) => void;
  /** Arquivos do "Abrir com Ayla", anexados uma vez (pelo `id`). */
  attach?: { id: number; paths: string[] };
  /** Bases escolhidas na barra lateral para a próxima mensagem. */
  bases?: { id: string; name: string }[];
  onBasesUsed?: () => void;
  onRemoveBase?: (id: string) => void;
  /** A conversa ficou vazia e foi apagada ao sair dela. */
  onDiscarded?: (id: string) => void;
};
type ResponseSource = 'local' | 'auto' | 'cloud';
const clock = (seconds: number) => `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
/** Altura da caixa de texto igual à do conteúdo, sem barra de rolagem. */
const fitHeight = (el: HTMLTextAreaElement) => { el.style.height = 'auto'; el.style.height = `${el.scrollHeight}px`; };

/** Editar uma pergunta (nova versão dela) ou gerar outra resposta: `id` da pergunta; `cut`, a
 * posição a partir da qual a tela troca; `files`, os anexos da pergunta original. */
type Branch = { kind: 'edit' | 'regenerate'; id: number; text: string; cut: number; files: FileInfo[] };
const withExtras = (m: Message): Message => m.extras ? { ...m, images: m.extras.images ?? m.images, sources: m.extras.sources ?? m.sources, designs: m.extras.designs ?? m.designs, ficha: m.extras.ficha ?? m.ficha, hand: m.extras.hand ?? m.hand, created: m.extras.created ?? m.created } : m;

/** Setas para alternar entre as versões de uma mensagem (‹ 2/3 ›). */
function Versions({ message, disabled, onPick }: { message: Message; disabled: boolean; onPick: (id: number) => void }) {
  const versions = message.versions ?? [];
  const at = message.id ? versions.indexOf(message.id) : -1;
  if (versions.length < 2 || at < 0) return null;
  return <span className="message-versions" role="group" aria-label="Versões">
    <button type="button" className="icon-button" aria-label="Versão anterior" disabled={disabled || at === 0} onClick={() => onPick(versions[at - 1])}><ChevronLeftIcon size={15} /></button>
    <span>{at + 1}/{versions.length}</span>
    <button type="button" className="icon-button" aria-label="Próxima versão" disabled={disabled || at === versions.length - 1} onClick={() => onPick(versions[at + 1])}><ChevronRightIcon size={15} /></button>
  </span>;
}

export default function Chat({ snapshot, refresh, project, conversation, onConversation, onBusy, onShowSidebar, active: visible, focusComposer, onOpenSettings, onOpenNote, attach, bases = [], onBasesUsed, onRemoveBase, onDiscarded }: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [editing, setEditing] = useState<{ index: number; text: string } | null>(null);
  // Editar ou gerar de novo à espera da escolha "nuvem ou local" do Automático.
  const askBranch = useRef<Branch | null>(null);
  const shownMessages = useRef(0);
  shownMessages.current = messages.length;
  const discarded = useRef(onDiscarded);
  discarded.current = onDiscarded;
  const [input, setInput] = useState(''); const [effort, setEffort] = useState<Effort>('');
  // Neste computador, o esforço escolhe entre resposta direta (leve) e raciocínio (alto).
  const kind: Capability | 'auto' = effort === 'low' ? 'text' : effort === 'high' ? 'reasoning' : 'auto';
  const [cloudModel, setCloudModel] = useState(''); const [cloudModels, setCloudModels] = useState<{ id: string; name: string }[]>([]);
  const [model, setModel] = useState(''); const [attachments, setAttachments] = useState<FileInfo[]>([]);
  const [adding, setAdding] = useState(false); const [dragging, setDragging] = useState(false); const [status, setStatus] = useState('');
  /** Escolha explícita de criar imagem; `null` = seguir a frase do pedido. */
  const [imageChoice, setImageChoice] = useState<boolean | null>(null);
  /** Botão "Criar design": a mensagem vira um design na Iris, montado neste computador. */
  const [designing, setDesigning] = useState(false);
  // "Usar o computador": a mensagem vai para o Hand (plano para aprovar), neste computador.
  const [handing, setHanding] = useState(false);
  const [titleDraft, setTitleDraft] = useState<string | null>(null);
  // O modo escolhido vale para as próximas conversas até ser trocado.
  const [source, setSource] = useState<ResponseSource>(snapshot.preferences.response_source || (snapshot.preferences.usage_mode === 'hybrid' ? 'auto' : 'local'));
  /** Automático em dúvida: a pessoa escolhe entre nuvem e este computador. */
  const [ask, setAsk] = useState<{ message: string; search: boolean; notes?: OfferedNote[]; kept?: KeptNote[]; manual?: boolean } | null>(null);
  const [skipNotes, setSkipNotes] = useState<string[]>([]);
  const [routeNotice, setRouteNotice] = useState('');
  /** A nuvem falhou no Automático (limite, sem conexão): dá para responder aqui com um clique. */
  const [retryLocal, setRetryLocal] = useState<string | null>(null);
  useEffect(() => { if (retryLocal !== null && (askBranch.current ? input.trim() !== '' : input.trim() !== retryLocal)) setRetryLocal(null); }, [retryLocal, input]);
  // Editar/Gerar de novo não devolvem a pergunta à caixa: aí o cartão só some se a pessoa escrever.
  useEffect(() => { if (ask && (askBranch.current ? input.trim() !== '' : input.trim() !== ask.message)) setAsk(null); }, [ask, input]);
  function chooseSource(next: ResponseSource) {
    setError(''); setSource(next); setSensitivity('uncertain'); setReviewed(false); setEffort(''); setAsk(null);
    void native('set_response_source', { source: next }).then(refresh).catch(e => setError(errorText(e)));
  }
  const [sensitivity, setSensitivity] = useState<Sensitivity>('uncertain');
  const [reviewed, setReviewed] = useState(false); const [error, setError] = useState('');
  const [routing, setRouting] = useState('');
  const [learned, setLearned] = useState<Fact[]>([]);
  const [busy, setBusy] = useState(false); const active = useRef<string | null>(null);
  const [listening, setListening] = useState<'idle' | 'recording' | 'transcribing'>('idle');
  const [seconds, setSeconds] = useState(0);
  const [options, setOptions] = useState(false);
  const [copied, setCopied] = useState<number | null>(null);
  const recorder = useRef<Recording | null>(null);
  const thread = useRef<HTMLDivElement | null>(null); const field = useRef<HTMLTextAreaElement | null>(null);
  const follow = useRef(true);
  /** Conversa criada pelo envio em andamento: já está na tela, não recarregar. */
  const created = useRef<string | null>(null);
  const mode = snapshot.preferences.usage_mode;
  // Pedidos sobre finanças usam a Moneta e respondem neste computador em qualquer modo (mesma lista do núcleo).
  const [localWords, setLocalWords] = useState<string[]>([]);
  useEffect(() => { void native<string[]>('organ_local_words').then(setLocalWords).catch(() => setLocalWords([])); }, [snapshot]);
  const projectContext = bases.some(b => b.id.startsWith('project-source:'));
  // Num pedido longo, "cria uma nota…" não prende a resposta aqui: a Ayla responde (podendo oferecer a
  // nuvem com as notas) e guarda a resposta como nota.
  const longRequest = input.trim().split(/\s+/).length >= 25;
  const organLocal = projectContext || localWords.some(word => input.toLowerCase().includes(word) && !(longRequest && /nota|athena/.test(word)));
  const provider = snapshot.preferences.cloud_provider;
  const destination = projectContext || mode === 'local_only' ? 'local' : mode === 'cloud_only' ? 'cloud' : source;
  const canResearch = destination === 'cloud' && !organLocal && !attachments.length && sensitivity !== 'sensitive' && Boolean(input.trim());
  const [decision, setDecision] = useState<{ draft: string; search: boolean } | null>(null);
  const pendingRoute = canResearch && decision?.draft !== input;
  useEffect(() => {
    if (!canResearch) return;
    let current = true;
    void native<boolean>('suggests_web_search', { message: input })
      .then(search => { if (current) setDecision({ draft: input, search }); })
      .catch(() => { if (current) setDecision({ draft: input, search: false }); });
    return () => { current = false; };
  }, [canResearch, input]);
  const research = canResearch && decision?.draft === input && decision.search;
  const cloud = !organLocal && !designing && !handing && destination === 'cloud';
  const searchAvailable = provider === 'chatgpt' || provider === 'openai';
  // Consultar a lista só quando a pessoa abrir a escolha do modelo.
  function loadCloudModels() {
    if (provider) void native<{ id: string; name: string }[]>('cloud_models', { provider })
      .then(setCloudModels).catch(() => setCloudModels([]));
  }
  useEffect(() => { setCloudModels([]); setCloudModel(''); setReviewed(false); }, [provider]);
  const needsSetup = mode !== 'cloud_only' && !snapshot.models.some(m => supports(m, 'text'));
  const suggested = snapshot.catalog.find(entry => entry.id === snapshot.recommended_model);
  const memoryEntry = snapshot.catalog.find(entry => entry.capabilities.includes('embedding'));
  const setup = snapshot.setup || { active: false };
  const start = () => { setError(''); void native('setup_start').catch(e => setError(errorText(e))).finally(() => void refresh()); setTimeout(() => void refresh(), 300); };
  const offerMemory = !needsSetup && !setup.active && snapshot.preferences.memory_auto && !snapshot.models.some(m => supports(m, 'embedding')) && memoryEntry;
  const conversationId = conversation?.id;
  const canHear = snapshot.models.some(m => supports(m, 'transcription'));
  // Primeiro preparo: conversa, memória (se ligada) e microfone (padrão), num só download.
  const firstSetup = [suggested, snapshot.preferences.memory_auto ? memoryEntry : undefined, ...(snapshot.preferences.voice !== false ? snapshot.catalog.filter(entry => entry.capabilities.includes('transcription')) : [])]
    .reduce((sum, entry) => sum + (entry && !entry.installed ? entry.download_bytes : 0), 0);
  // Criar imagem só quando a pessoa pede: botão ou frase explícita ("gere uma imagem de…").
  const asksImage = asksForImage(input);
  const image = !projectContext && !designing && !handing && (imageChoice ?? asksImage) && mode !== 'local_only';
  // Referências: memórias públicas relevantes, listadas na revisão; só as marcadas vão junto.
  const [references, setReferences] = useState<Reference[]>([]); const [dropped, setDropped] = useState<string[]>([]);
  const reviewing = cloud && !pendingRoute && !research && !image && input.trim().length >= 3;
  useEffect(() => {
    if (!reviewing) return;
    let alive = true;
    const timer = setTimeout(() => void native<Reference[]>('cloud_references', { project: project?.id ?? null, message: input }).then(list => { if (alive) setReferences(list); }).catch(() => { if (alive) setReferences([]); }), 600);
    return () => { alive = false; clearTimeout(timer); };
  }, [reviewing, input, project?.id]);
  const shownReferences = reviewing ? references : [];
  const kept = shownReferences.filter(r => !dropped.includes(r.id));
  const imageProvider = snapshot.preferences.image_provider;
  const canSend = image
    ? !busy && !adding && listening === 'idle' && Boolean(input.trim()) && reviewed && Boolean(imageProvider && snapshot.preferences.image_model)
    : !busy && !adding && listening === 'idle' && Boolean(input.trim() || attachments.length) && !pendingRoute && (!cloud || reviewed && (!research || searchAvailable) && (mode !== 'per_conversation' || conversation?.cloud_consent));
  const visual = attachments.some(file => file.kind === 'image' || file.kind === 'gif' || file.kind === 'video');

  useEffect(() => {
    if (!conversationId) { setLearned([]); return; }
    const load = () => native<Fact[]>('conversation_memories', { id: conversationId }).then(setLearned).catch(() => setLearned([]));
    void load();
    const stop = listen<{ conversation: string }>('memory-updated', event => { if (event.payload.conversation === conversationId) void load(); });
    return () => { void stop.then(unlisten => unlisten()); };
  }, [conversationId]);

  async function forget(fact: Fact) {
    try { await native('memory_delete', { id: fact.id }); setLearned(prev => prev.filter(f => f.id !== fact.id)); } catch (e) { setError(errorText(e)); }
  }

  useEffect(() => () => { if (active.current) void native('cancel_job', { requestId: active.current }); recorder.current?.stop(); }, []);
  useEffect(() => { onBusy(busy); }, [busy, onBusy]);

  // Saindo de uma conversa sem nenhuma mensagem (ex.: o envio falhou), ela é apagada; o núcleo
  // confere de novo que não há mensagens nem arquivos.
  useEffect(() => {
    if (!conversationId) return;
    return () => {
      if (shownMessages.current > 0 || active.current) return;
      void native<boolean>('conversation_discard_empty', { id: conversationId }).then(gone => { if (gone) discarded.current?.(conversationId); }).catch(() => {});
    };
  }, [conversationId]);
  // Abre a conversa escolhida na barra do espaço (ou limpa a tela para uma nova).
  useEffect(() => {
    if (created.current === conversationId) { created.current = null; return; }
    setError(''); setReviewed(false); setRouting(''); follow.current = true;
    if (!conversationId) { setMessages([]); return; }
    let current = true;
    native<Message[]>('conversation_messages', { id: conversationId }).then(items => { if (current) setMessages(items.map(withExtras)); }).catch(e => { if (current) setError(errorText(e)); });
    return () => { current = false; };
  }, [conversationId]);

  useEffect(() => { setReviewed(false); }, [snapshot.system_prompt, provider, input, research]);

  useLayoutEffect(() => {
    if (visible && focusComposer != null) field.current?.focus({ preventScroll: true });
  }, [visible, focusComposer]);

  // Acompanha a resposta enquanto a pessoa estiver no fim da conversa.
  useLayoutEffect(() => { const el = thread.current; if (el && follow.current) el.scrollTop = el.scrollHeight; }, [messages, learned]);
  useLayoutEffect(() => {
    const el = field.current;
    if (!el || !visible) return;
    if (!input) {
      // O campo vazio usa a altura natural de rows={1}.
      el.style.height = '';
      return;
    }
    // Mede o texto sem incluir a altura anterior do campo.
    function fit() {
      if (!el || !el.clientWidth) return;
      el.style.height = '0px';
      el.style.height = `${el.scrollHeight}px`;
    }
    fit();
    let width = el.clientWidth;
    const observer = new ResizeObserver(() => {
      if (el.clientWidth === width) return;
      fit();
      width = el.clientWidth;
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [input, visible]);
  useEffect(() => {
    if (listening !== 'recording') return;
    setSeconds(0); const timer = setInterval(() => setSeconds(s => s + 1), 1000);
    return () => clearInterval(timer);
  }, [listening]);

  // Arquivos ficam neste computador: cópia guardada pela Ayla, nunca enviada à nuvem.
  async function attachPaths(paths: string[]) {
    if (!paths.length) return;
    if (cloud) { setError('Arquivos ficam neste computador. Responda localmente para usá-los.'); return; }
    setError(''); setAdding(true);
    let count = attachments.length;
    try {
      for (const path of paths) {
        if (count >= 5) { setError('Envie até 5 arquivos por mensagem.'); break; }
        const file = await native<FileInfo>('add_file', { path, project: project?.id ?? null });
        count += 1; setAttachments(prev => [...prev, file]);
      }
    } catch (e) { setError(errorText(e)); } finally { setAdding(false); }
  }
  // A pessoa escolhe uma região da tela; a captura vira anexo local. Retorna null se cancelou.
  async function captureScreen(): Promise<FileInfo | null> {
    if (cloud) { setError('A tela fica neste computador. Responda localmente para a Ayla vê-la.'); return null; }
    if (attachments.length >= 5) { setError('Envie até 5 arquivos por mensagem.'); return null; }
    setError(''); setAdding(true);
    try { return await native<FileInfo | null>('capture_screen', { project: project?.id ?? null }); }
    catch (e) { setError(errorText(e)); return null; } finally { setAdding(false); }
  }
  const attachLatest = useRef(attachPaths);
  useEffect(() => { attachLatest.current = attachPaths; });
  const attached = useRef(0);
  useEffect(() => {
    if (!attach || attach.id === attached.current) return;
    attached.current = attach.id;
    void attachLatest.current(attach.paths).then(() => field.current?.focus());
  }, [attach]);
  // Arrastar arquivos para a janela anexa à conversa visível.
  useEffect(() => {
    if (!visible) return;
    let stop: (() => void) | undefined; let gone = false;
    void getCurrentWebview().onDragDropEvent(event => {
      const kind = event.payload.type;
      if (kind === 'enter' || kind === 'over') setDragging(true);
      else if (kind === 'leave') setDragging(false);
      else if (kind === 'drop') { setDragging(false); void attachLatest.current(event.payload.paths); }
    }).then(unlisten => { if (gone) unlisten(); else stop = unlisten; }).catch(() => {});
    return () => { gone = true; stop?.(); setDragging(false); };
  }, [visible]);
  function removeAttachment(file: FileInfo) {
    setAttachments(prev => prev.filter(item => item.id !== file.id));
    void native('remove_file', { id: file.id }).catch(() => {});
  }
  async function createImage() {
    const prompt = input.trim();
    if (!prompt || active.current) return;
    const id = taskId(); active.current = id; setBusy(true); setError(''); follow.current = true;
    let current = conversation;
    if (!current) {
      try { current = await native<Conversation>('create_conversation', { project: project?.id ?? null }); created.current = current.id; onConversation(current); }
      catch (e) { setError(errorText(e)); active.current = null; setBusy(false); return; }
    }
    setInput(''); setImageChoice(null);
    setMessages(prev => [...prev, { role: 'user', content: prompt, complete: true }, { role: 'assistant', content: '', complete: false }]);
    setStatus('Criando a imagem na nuvem…');
    try {
      const reply = await native<{ content: string; attachments: FileInfo[] }>('generate_image', { request: { id, conversation: current.id, prompt, sensitivity, reviewed } });
      setMessages(prev => prev.map((m, i) => i === prev.length - 1 ? { role: 'assistant', content: reply.content, complete: true, origin: 'cloud', attachments: reply.attachments } : m));
    } catch (e) {
      // Nada foi salvo: devolve o pedido para a caixa de mensagem.
      setError(errorText(e)); setMessages(prev => prev.slice(0, -2)); setInput(prompt); setImageChoice(true);
    } finally { active.current = null; setBusy(false); setReviewed(false); setStatus(''); await refresh(); }
  }
  /** Caminho da conversa como o núcleo guarda (com ids e versões), depois de enviar ou alternar. */
  async function reload(id: string, keepNotice = false) {
    try {
      const items = (await native<Message[]>('conversation_messages', { id })).map(withExtras);
      // O aviso de rota da resposta que acabou de chegar não fica guardado: continua na tela.
      setMessages(prev => {
        const notice = keepNotice ? prev[prev.length - 1]?.notice : undefined;
        return notice && items.length ? items.map((m, i) => i === items.length - 1 ? { ...m, notice } : m) : items;
      });
    } catch (e) { setError(errorText(e)); }
  }
  /** Mostra outra versão de uma mensagem (‹ 2/3 ›). */
  async function pickVersion(message: number) {
    if (!conversation || active.current) return;
    try { setMessages((await native<Message[]>('conversation_select', { id: conversation.id, message })).map(withExtras)); } catch (e) { setError(errorText(e)); }
  }
  function submitEdit(event: FormEvent) {
    event.preventDefault();
    const target = editing && messages[editing.index];
    if (!editing || !target?.id || !editing.text.trim()) return;
    setEditing(null);
    void send(undefined, undefined, { kind: 'edit', id: target.id, text: editing.text, cut: editing.index, files: target.attachments ?? [] });
  }
  /** `branch`: nova versão de uma pergunta (editar) ou outra resposta para ela (gerar de novo); as
   * mensagens a partir de `cut` saem da tela e a resposta nova entra no lugar. */
  async function send(spokenText?: string, choice?: 'cloud' | 'local', branchArg?: Branch) {
    const branch = branchArg ?? (choice ? askBranch.current : null) ?? null;
    askBranch.current = null;
    if (image && spokenText === undefined && !branch) { await createImage(); return; }
    const typed = branch ? branch.text.trim() : (spokenText ?? input).trim();
    const text = typed || (!branch && attachments.length ? (attachments.length === 1 ? 'Veja o arquivo anexado.' : 'Veja os arquivos anexados.') : '');
    if (!text || active.current || adding) return;
    // "O que é isso na tela?": a Ayla pede a região da tela antes de responder.
    let shot: FileInfo | null = null;
    if (!branch && spokenText === undefined && asksAboutScreen(text) && !attachments.some(file => file.kind === 'image')) {
      shot = await captureScreen();
      if (!shot) return;
    }
    const id = taskId(); active.current = id; setBusy(true); setError(''); follow.current = true;
    let current = conversation;
    if (!current) {
      try { current = await native<Conversation>('create_conversation', { project: project?.id ?? null }); created.current = current.id; onConversation(current); }
      catch (e) { setError(errorText(e)); active.current = null; setBusy(false); return; }
    }
    const message = text; if (spokenText === undefined && !branch) setInput('');
    const sent = branch ? branch.files : shot ? [...attachments, shot] : attachments; if (!branch) setAttachments([]);
    const offered = choice === 'cloud' ? (ask?.notes ?? []).filter(note => !skipNotes.includes(note.path)) : []; setAsk(null); setRouteNotice(''); setRetryLocal(null);
    const question: Message = { role: 'user', content: message, complete: true, attachments: sent };
    const pending: Message = { role: 'assistant', content: '', complete: false };
    setMessages(prev => branch ? [...prev.slice(0, branch.cut), ...(branch.kind === 'edit' ? [question] : []), pending] : [...prev, question, pending]);
    // Automático: a Ayla classifica neste computador antes do envio e pergunta quando houver dúvida.
    let autoRoute: ChatRequest['auto_route'] = null;
    // Sem internet, o Automático responde neste computador.
    const automaticCloud = destination === 'auto' && !designing && !handing && Boolean(provider) && !sent.length && !organLocal && sensitivity !== 'sensitive'
      && (mode !== 'per_conversation' || Boolean(current.cloud_consent)) && navigator.onLine;
    // Gerar de novo/editar: a nota da versão anterior não conta como "o último".
    const versionOf = { conversation: current.id, editOf: branch?.kind === 'edit' ? branch.id : null, regenerate: branch?.kind === 'regenerate' ? branch.id : null };
    /** Devolve o pedido à caixa de texto e mostra o cartão de escolha. */
    const holdFor = async (next: NonNullable<typeof ask>) => {
      if (branch) { askBranch.current = branch; await reload(current.id); } else { setMessages(prev => prev.slice(0, -2)); setAttachments(sent); setInput(message); }
      setAsk(next); setSkipNotes([]); active.current = null; setBusy(false); setStatus('');
    };
    if (automaticCloud && choice) autoRoute = choice === 'cloud' && ask?.search ? 'search' : choice;
    else if (automaticCloud) {
      setStatus('Verificando a privacidade do pedido…');
      try {
        const check = await native<{ route: 'local' | 'cloud' | 'search' | 'ask' | 'notes'; search: boolean; notes?: OfferedNote[]; kept?: KeptNote[]; notice?: string }>('automatic_check', { message, project: project?.id ?? null, ...versionOf });
        if (check.route === 'ask' || check.route === 'notes') { await holdFor({ message, search: check.search, notes: check.notes, kept: check.kept }); return; }
        if (check.notice) setRouteNotice(check.notice);
        autoRoute = check.route;
      } catch { autoRoute = 'local'; }
    } else if (cloud && !choice && !sent.length && !designing) {
      // Nuvem manual: as notas que o pedido usa só vão junto se a pessoa aceitar.
      setStatus('Procurando notas para o pedido…');
      const check = await native<{ notes: OfferedNote[]; kept?: KeptNote[]; notice?: string }>('cloud_notes_check', { message, project: project?.id ?? null, ...versionOf }).catch(() => ({ notes: [] as OfferedNote[], kept: undefined, notice: undefined }));
      if (check.notes.length) { await holdFor({ message, search: false, notes: check.notes, kept: check.kept, manual: true }); return; }
      if (check.notice) setRouteNotice(check.notice);
    }
    const request: ChatRequest = { id, conversation: current.id, message, personality_preview: cloud ? snapshot.system_prompt : null, capability: kind === 'auto' ? null : kind, model: destination === 'auto' ? null : model || null, attachments: branch ? [] : sent.map(file => file.id), edit_of: branch?.kind === 'edit' ? branch.id : null, regenerate: branch?.kind === 'regenerate' ? branch.id : null, sensitivity, reviewed, request_cloud: cloud, automatic: destination === 'auto', auto_route: autoRoute, web_search: Boolean(research && cloud), cloud_model: cloud ? cloudModel || null : null, effort: destination === 'local' ? null : effort || null, references: cloud ? kept : [], bases: bases.map(b => b.id), design: designing, hand: handing, notes: offered.map(note => note.path) };
    if (designing) setDesigning(false);
    if (handing) setHanding(false);
    if (bases.length) onBasesUsed?.();
    let receivedText = false;
    try {
      await nativeTask<void>('chat', { request }, event => {
        if (event.type === 'origin') setMessages(prev => prev.map((m, i) => i === prev.length - 1 ? { ...m, origin: event.origin } : m));
        if (event.type === 'routing') { setRouting(event.text || ''); setMessages(prev => prev.map((m, i) => i === prev.length - 1 ? { ...m, notice: event.text || '' } : m)); }
        if (event.type === 'images') setMessages(prev => prev.map((m, i) => i === prev.length - 1 ? { ...m, images: { query: event.query ?? '', items: event.items ?? [], links: event.links ?? [] } } : m));
        if (event.type === 'note_created') { const item = (event as unknown as { item?: CreatedNote }).item; if (item) setMessages(prev => prev.map((m, i) => i === prev.length - 1 ? { ...m, created: [...(m.created ?? []), item] } : m)); }
        if (event.type === 'hand') { const item = (event as unknown as { item?: HandCard }).item; if (item) setMessages(prev => prev.map((m, i) => i === prev.length - 1 ? { ...m, hand: item } : m)); }
        if (event.type === 'ficha') { const item = (event as unknown as { item?: FichaCard }).item; if (item) setMessages(prev => prev.map((m, i) => i === prev.length - 1 ? { ...m, ficha: item } : m)); }
        if (event.type === 'designs') { const items = (event as unknown as { items?: DesignCard[] }).items ?? []; setMessages(prev => prev.map((m, i) => i === prev.length - 1 ? { ...m, designs: items } : m)); }
        if (event.type === 'suggestion') setError(event.text || 'A Ayla sugeriu revisar o pedido para cloud.');
        if (event.type === 'memory' && event.facts?.length) setLearned(prev => [...prev.filter(f => !event.facts!.some(n => n.id === f.id)), ...event.facts!]);
        if (event.type === 'memory_error') setError(`Não consegui atualizar a memória: ${event.text}`);
        if (event.type === 'status') setStatus(event.text || '');
        if (event.type === 'sources' && event.files?.length) setMessages(prev => { const next = [...prev]; const last = next[next.length - 1]; const known = last.sources || []; next[next.length - 1] = { ...last, sources: [...known, ...event.files!.filter(f => !known.some(k => k.id === f.id))] }; return next; });
        if (event.type === 'token' && event.text) { receivedText = true; setStatus(''); }
        if (event.type === 'token' && event.text) setMessages(prev => { const next = [...prev]; const last = next[next.length - 1]; next[next.length - 1] = { ...last, content: last.content + event.text }; return next; });
      });
      setMessages(prev => prev.map((m, index) => index === prev.length - 1 ? { ...m, complete: true } : m));
      // Ids e versões vêm do núcleo (para editar e gerar de novo).
      await reload(current.id, true);
    } catch (e) {
      setError(errorText(e));
      if (branch) {
        await reload(current.id);
        // A nuvem falhou ao gerar de novo/editar: dá para refazer aqui, na mesma versão.
        if (!receivedText && (autoRoute === 'cloud' || autoRoute === 'search')) { askBranch.current = branch; setRetryLocal(message); }
      }
      else if (!receivedText && (research || destination === 'auto')) {
        setMessages(prev => prev.slice(0, -2));
        setAttachments(sent);
        setInput(message);
        if (autoRoute === 'cloud' || autoRoute === 'search') setRetryLocal(message);
      }
    } finally { active.current = null; setBusy(false); setReviewed(false); setStatus(''); await refresh(); }
  }
  async function listen_() {
    if (listening === 'recording') { recorder.current?.stop(); return; }
    if (listening !== 'idle' || busy) return;
    setError('');
    // O que já estava escrito continua; a fala entra depois dele.
    const before = input.trim();
    const join = (text: string) => before ? `${before} ${text}` : text;
    try {
      setListening('recording');
      const session = await recordWav(); recorder.current = session;
      // Prévia: transcreve o que já foi falado a cada ~1,5 s e mostra no campo de texto.
      let previewing = false;
      const preview = setInterval(() => {
        const partial = previewing ? null : session.peek();
        if (!partial) return;
        previewing = true;
        void native<{ text: string | null }>('transcribe_preview', { requestId: taskId(), audio: partial })
          .then(result => { if (result.text && recorder.current === session) setInput(join(result.text)); })
          .catch(() => {})
          .finally(() => { previewing = false; });
      }, 1500);
      const audio = await session.result.finally(() => clearInterval(preview)); recorder.current = null;
      setListening('transcribing');
      const id = taskId();
      const result = await native<{ text?: string }>('voice_task', { request: { id, text: '', sensitivity: 'sensitive', reviewed: false, request_cloud: false, cloud_model: null }, audio });
      setListening('idle');
      const heard = (result.text || '').trim();
      // A fala fica na caixa de texto para revisar e enviar quando quiser (nunca é enviada sozinha).
      setInput(heard ? join(heard) : before);
      if (heard) field.current?.focus(); else setError('Não entendi o áudio. Tente falar de novo.');
      if (session.reachedLimit()) setError(`A gravação vai até ${RECORD_LIMIT_MS / 60000} minutos: parei aí e transcrevi o que você falou. Toque no microfone para continuar.`);
    } catch (e) { recorder.current = null; setListening('idle'); setInput(before); setError(errorText(e)); }
  }
  async function consent(allowed: boolean) {
    try {
      const current = conversation ?? await native<Conversation>('create_conversation', { project: project?.id ?? null });
      await native('conversation_consent', { id: current.id, allowed });
      onConversation({ ...current, cloud_consent: allowed });
    }
    catch (e) { setError(errorText(e)); }
  }
  // Colar (Ctrl+V) imagens e outros arquivos: viram anexos locais, como os escolhidos.
  function paste(event: ClipboardEvent<HTMLTextAreaElement>) {
    const pasted = Array.from(event.clipboardData.items).filter(item => item.kind === 'file').map(item => item.getAsFile()).filter((file): file is File => file !== null);
    if (!pasted.length) return;
    event.preventDefault();
    if (cloud) { setError('Arquivos ficam neste computador. Responda localmente para usá-los.'); return; }
    setError(''); setAdding(true);
    void (async () => {
      try {
        for (const file of pasted.slice(0, Math.max(0, 5 - attachments.length))) {
          const data = base64(new Uint8Array(await file.arrayBuffer()));
          const saved = await native<FileInfo>('add_file_data', { name: file.name || 'colado.png', data, project: project?.id ?? null });
          setAttachments(prev => [...prev, saved]);
        }
      } catch (e) { setError(errorText(e)); } finally { setAdding(false); }
    })();
  }
  // Primeira abertura: 100% local ou também nuvem (com contas conectadas em Configurações).
  async function chooseCloud(useCloud: boolean) {
    try {
      await savePreferences({ ...snapshot.preferences, cloud_asked: true, usage_mode: useCloud ? 'explicit_request' : 'local_only' });
      await refresh();
      if (useCloud) onOpenSettings?.('nuvem');
    } catch (e) { setError(errorText(e)); }
  }
  async function saveTitle() {
    const title = (titleDraft ?? '').trim(); setTitleDraft(null);
    if (!conversation || !title || title === conversation.title) return;
    try { await native('rename_conversation', { id: conversation.id, title }); onConversation({ ...conversation, title }); } catch (e) { setError(errorText(e)); }
  }
  // A caixa de edição abre do tamanho da mensagem inteira e cresce enquanto se digita.
  const fitEdit = useCallback((el: HTMLTextAreaElement | null) => { if (el) { fitHeight(el); el.setSelectionRange(el.value.length, el.value.length); } }, []);
  async function copy(text: string, index: number) {
    try { await navigator.clipboard.writeText(text); setCopied(index); setTimeout(() => setCopied(c => c === index ? null : c), 1500); }
    catch { setError('Não consegui copiar o texto.'); }
  }

  return <>
    <main className="chat-main">
      {dragging && <div className="drop-overlay" aria-hidden="true"><FileIcon size={28} /><span>Solte para anexar · fica neste computador</span></div>}
      <header className="chat-header">
        {onShowSidebar && <button className="icon-button" aria-label="Mostrar a barra lateral" onClick={onShowSidebar}><SidebarIcon /></button>}
        {titleDraft !== null && conversation
          ? <form className="title-edit" onSubmit={e => { e.preventDefault(); void saveTitle(); }}><input autoFocus aria-label="Título da conversa" value={titleDraft} maxLength={120} onChange={e => setTitleDraft(e.target.value)} onBlur={() => void saveTitle()} onKeyDown={e => { if (e.key === 'Escape') { e.preventDefault(); setTitleDraft(null); } }} /></form>
          : <h1 onDoubleClick={() => { if (conversation) setTitleDraft(conversation.title); }}>{conversation?.title || 'Nova conversa'}</h1>}
        {conversation && titleDraft === null && <button className="icon-button" aria-label="Renomear conversa" title="Renomear" onClick={() => setTitleDraft(conversation.title)}><PencilIcon size={16} /></button>}
        <span className="chip"><span className="dot" aria-hidden="true" />{destination === 'auto' ? 'Automático' : cloud ? 'Nuvem' : 'Local'}</span>
      </header>

      <div className="messages" ref={thread} aria-live="polite" onScroll={e => { const el = e.currentTarget; follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80; }}>
        <div className="thread">
          {!messages.length && !needsSetup && !setup.active && <div className="empty-state"><span className="orb large" aria-hidden="true" /><p>{project ? `O que vamos fazer em ${project.name}?` : 'O que vamos fazer hoje?'}</p></div>}
          {!messages.length && (needsSetup || setup.active) && <section className="setup-card" aria-label="Preparar a Ayla">
            <h2>{setup.active ? 'Preparando a Ayla neste computador' : 'Vamos preparar a Ayla neste computador'}</h2>
            {setup.active ? <><p>Baixando os modelos padrão para a Ayla funcionar sem internet. Você pode usar outras telas enquanto isso.</p>
              <div role="status" className="setup-progress"><progress max={setup.total || 1} value={setup.received || 0} aria-label="Progresso do preparo" /><span>{setup.label} · {formatBytes(setup.received || 0)} de {formatBytes(setup.total || 0)}</span><button onClick={() => void native('cancel_job', { requestId: 'auto-setup' }).then(refresh)}>Interromper</button></div></>
              : suggested ? <><p>{setup.error ? `O preparo não terminou: ${setup.error}` : setup.cancelled ? 'Preparo interrompido.' : 'A Ayla precisa de um modelo de IA para conversar sem internet.'} Recomendado para o seu computador: <b>{suggested.name}</b>.</p><button className="primary-action" onClick={start}>Preparar a Ayla · {formatBytes(firstSetup)}</button></>
              : <p>Os modelos recomendados não cabem na memória livre agora (disponível: {formatBytes(snapshot.memory_budget_bytes)}). Feche alguns programas, escolha uma preferência de uso maior em Configurações ou importe um modelo menor nas opções avançadas.</p>}
          </section>}
          {!messages.length && !project && !snapshot.preferences.cloud_asked && <section className="setup-card" aria-label="Local ou nuvem">
            <h2>Como a Ayla deve funcionar?</h2>
            <p>Por padrão, tudo fica neste computador. Se quiser, conecte suas contas de IA na nuvem (ChatGPT, Claude, Gemini e outras) para os pedidos que você revisar e liberar. Dá para mudar depois em Configurações.</p>
            <div className="button-row"><button className="primary-action" onClick={() => void chooseCloud(false)}>Ficar 100% local</button><button onClick={() => void chooseCloud(true)}>Também usar a nuvem</button></div>
          </section>}
          {!messages.length && offerMemory && <section className="setup-card" aria-label="Ativar memória"><h2>Quer que a Ayla lembre de você?</h2><p>Com a memória ativada, a Ayla guarda neste computador o que for importante nas conversas, como preferências, pessoas e projetos. Você vê e apaga tudo em Configurações.</p><button className="primary-action" onClick={start}>Ativar memória · {formatBytes(memoryEntry.download_bytes)}</button></section>}

          {messages.map((m, i) => m.role === 'user'
            ? editing?.index === i
              ? <form key={i} className="message user editing" onSubmit={submitEdit}>
                  <textarea ref={fitEdit} aria-label="Editar a mensagem" autoFocus value={editing.text} rows={1}
                    onChange={e => { fitHeight(e.currentTarget); setEditing({ index: i, text: e.target.value }); }}
                    onKeyDown={e => { if (e.key === 'Escape') setEditing(null); if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) submitEdit(e); }} />
                  <div className="row-actions"><button type="button" className="ghost-button" onClick={() => setEditing(null)}>Cancelar</button><button className="primary-action" disabled={!editing.text.trim() || busy}>Enviar</button></div>
                </form>
              : <Fragment key={i}>
                  <div className="message user">{m.attachments?.length ? <span className="message-files">{m.attachments.map(file => file.kind === 'image' || file.kind === 'gif' ? <FilePreview key={file.id} file={file} /> : <span key={file.id} className="file-chip"><FileIcon size={14} />{file.name}</span>)}</span> : null}{m.content}</div>
                  {m.id && <div className="message-actions user-actions">
                    <Versions message={m} disabled={busy} onPick={id => void pickVersion(id)} />
                    {m.content && <button className="ghost-button" onClick={() => void copy(m.content, i)}><CopyIcon size={15} />{copied === i ? 'Copiado' : 'Copiar'}</button>}
                    <button className="ghost-button" disabled={busy} title="Editar e enviar de novo a partir daqui" onClick={() => setEditing({ index: i, text: m.content })}><PencilIcon size={15} />Editar</button>
                  </div>}
                </Fragment>
            : <div key={i} className="message assistant">
                <span className="orb small" aria-hidden="true" />
                <div className="assistant-body">
                  {m.content ? <Markdown text={m.content} /> : busy && i === messages.length - 1 ? <p className="thinking" role="status">{status || 'Pensando…'}</p> : null}
                  {m.attachments?.map(file => <FilePreview key={file.id} file={file} />)}
                  {m.designs?.length ? <ChatDesigns items={m.designs} /> : null}
                  {m.hand ? <ChatHand item={m.hand} /> : null}
                  {m.ficha ? <ChatFicha item={m.ficha} conversa={conversationId} onOpen={onOpenNote} /> : null}
                  {m.images && <ImageResults results={m.images} project={project?.id ?? null} onError={setError} />}
                  {m.created?.length && onOpenNote ? <p className="message-created">{m.created.map(item => <button key={item.note} type="button" className="file-chip link-chip" title={item.note} onClick={() => onOpenNote(item.note)}><FileIcon size={14} />Abrir “{item.title}” na Athena</button>)}</p> : null}
                  {m.sources?.length ? <p className="message-sources"><span>Fontes</span>{m.sources.map(source => {
                    const label = <><FileIcon size={14} />{source.name}{source.date ? ` · ${source.date}` : ''}{source.pages.length ? ` · p. ${source.pages.join(', ')}` : ''}</>;
                    if (source.note && onOpenNote) return <button key={source.id} type="button" className="file-chip link-chip" title="Abrir na Athena" onClick={() => onOpenNote(source.note!)}>{label}</button>;
                    return source.url
                      ? <button key={source.id} type="button" className="file-chip link-chip" title={source.url} onClick={() => void native('open_web', { url: source.url }).catch(e => setError(errorText(e)))}>{label}</button>
                      : <span key={source.id} className="file-chip">{label}</span>;
                  })}</p> : null}
                  {m.notice && m.complete && <p className="message-notice">{m.notice}</p>}
                  {!m.complete && m.content && !busy && <small className="incomplete">Resposta incompleta</small>}
                  {m.origin && m.content && <small className="reply-origin">{m.origin === 'cloud_search' ? 'Busca na nuvem' : 'Resposta na nuvem'}</small>}
                  {(m.id || (m.complete && m.content)) && <div className="message-actions">
                    <Versions message={m} disabled={busy} onPick={id => void pickVersion(id)} />
                    {m.id && messages[i - 1]?.role === 'user' && messages[i - 1].id && <button className="ghost-button" disabled={busy} title="Gerar outra resposta para esta pergunta"
                      onClick={() => { const q = messages[i - 1]; void send(undefined, undefined, { kind: 'regenerate', id: q.id!, text: q.content, cut: i, files: q.attachments ?? [] }); }}><RefreshIcon size={15} />Gerar de novo</button>}
                    {m.complete && m.content && <button className="ghost-button" onClick={() => void copy(m.content, i)}><CopyIcon size={16} />{copied === i ? 'Copiado' : 'Copiar'}</button>}
                  </div>}
                </div>
              </div>)}
          {learned.map(fact => <div key={fact.id} className="memory-note"><span>Ayla lembrou: {fact.content}</span><Confirm className="link-button" question="Apagar esta memória?" confirm="Apagar" onConfirm={() => void forget(fact)}>Desfazer</Confirm></div>)}
        </div>
      </div>

      <div className="composer-area">
        {error && <p className="chat-error" role="alert">{error}</p>}
        {retryLocal !== null && !busy && <div className="route-ask" role="group" aria-label="Responder neste computador">
          <p>Posso responder neste computador, com o modelo local e as suas notas.</p>
          <div className="button-row"><button type="button" onClick={() => void send(undefined, 'local')}>Responder neste computador</button></div>
        </div>}
        {routeNotice && !busy && !ask && <p className="route-notice" role="status">{routeNotice}</p>}
        {ask && !busy && (ask.notes?.length ? <div className="route-ask" role="group" aria-label="Enviar notas à nuvem">
          <p>{ask.notes.length === 1 ? 'Achei esta nota para o pedido' : 'Achei estas notas para o pedido'} e conferi neste computador: nada sensível. {ask.manual ? 'Envio junto à nuvem?' : `Posso ${ask.search ? 'pesquisar na internet' : 'responder pela nuvem'} enviando seu pedido com as notas marcadas, ou manter tudo neste computador.`}</p>
          <ul className="route-ask-notes">{ask.notes.map(note => <li key={note.path}><label title={note.path}>
            <input type="checkbox" checked={!skipNotes.includes(note.path)} onChange={e => setSkipNotes(prev => e.target.checked ? prev.filter(p => p !== note.path) : [...prev, note.path])} />{note.title}</label></li>)}</ul>
          {ask.kept?.map(note => <p key={note.title} className="route-ask-hint">A nota “{note.title}” fica neste computador ({note.reason}).</p>)}
          {ask.notes.some(note => note.cut) && <p className="route-ask-hint">Notas longas vão com o começo e o final (até 12 mil caracteres); o meio fica aqui.</p>}
          <div className="button-row">
            <button type="button" onClick={() => void send(undefined, 'cloud')}>{ask.notes.every(note => skipNotes.includes(note.path)) ? (ask.manual ? 'Enviar só o pedido' : 'Usar a nuvem sem notas') : ask.notes.length - skipNotes.length === 1 ? 'Enviar com a nota' : 'Enviar com as notas'}</button>
            <button type="button" onClick={() => void send(undefined, 'local')}>{ask.manual ? 'Enviar só o pedido' : 'Manter neste computador'}</button>
          </div>
        </div> : <div className="route-ask" role="group" aria-label="Onde responder">
          <p>Não tenho certeza se este pedido tem algo pessoal. {ask.search ? 'Posso pesquisar na internet' : 'Posso responder pela nuvem'} ou manter tudo neste computador.</p>
          <div className="button-row">
            <button type="button" onClick={() => void send(undefined, 'cloud')}>{ask.search ? 'Pesquisar na internet' : 'Usar a nuvem'}</button>
            <button type="button" onClick={() => void send(undefined, 'local')}>Manter neste computador</button>
          </div>
        </div>)}
        {options && <div className="response-options" id="response-options" role="group" aria-label="Ajustes desta resposta">
          {routing && <p role="status">Última resposta: {routing}</p>}
          <label>Modo<select value={destination} disabled={busy || mode === 'local_only' || mode === 'cloud_only'} onChange={e => chooseSource(e.target.value as ResponseSource)}><option value="local">Neste computador</option><option value="auto">Automático</option><option value="cloud">Na nuvem{provider ? '' : ' · conecte uma conta'}</option></select></label>
          {destination !== 'auto' && (cloud
            ? <label>Modelo da nuvem<select value={cloudModel} onFocus={loadCloudModels} disabled={busy || !provider} onChange={e => setCloudModel(e.target.value)}><option value="">Automático</option>{cloudModels.map(m => <option key={m.id} value={m.id}>{m.name}</option>)}</select></label>
            : <label>Modelo<select value={model} disabled={busy} onChange={e => setModel(e.target.value)}><option value="">Escolha automática</option>{snapshot.models.filter(m => visual ? supports(m, 'vision') : kind === 'auto' ? (['text', 'reasoning', 'vision'] as Capability[]).some(k => supports(m, k)) : supports(m, kind)).map(m => <option key={m.id} value={m.id}>{m.name}</option>)}</select></label>)}
          <label>Esforço<select value={effort} disabled={busy} onChange={e => { setEffort(e.target.value as Effort); if (!cloud) setModel(''); }}>
            <option value="">Automático</option><option value="low">{cloud ? 'Leve' : 'Leve · resposta direta'}</option><option value="medium">Médio</option><option value="high">{cloud ? 'Alto' : 'Alto · raciocínio'}</option>
          </select></label>
          {cloud && effort && <p className="row-hint">O esforço vale para modelos que raciocinam; outros podem recusar o pedido.</p>}
          {mode === 'local_only' && <p className="row-hint">Nuvem desligada. <button type="button" className="link-button" onClick={() => onOpenSettings?.('nuvem')}>Ativar em Configurações › Nuvem</button></p>}
          {mode === 'per_conversation' && !cloud && (conversation || destination === 'auto') && <label className="check"><input type="checkbox" checked={conversation?.cloud_consent ?? false} disabled={busy} onChange={e => void consent(e.target.checked)} />Permitir nuvem nesta conversa, respeitando a privacidade de cada pedido</label>}
        </div>}
        {image && <div className="cloud-review" aria-label="Revisão do pedido de imagem">
          {imageProvider && snapshot.preferences.image_model ? <>
            <b>Pedido da imagem que será enviado</b><pre>{input || 'Descreva a imagem.'}</pre>
            <p>Só este texto vai para a plataforma de imagens ({snapshot.preferences.image_model}). Histórico, memórias e anexos ficam neste computador.</p>
            <label>Privacidade do pedido<select value={sensitivity} disabled={busy} onChange={e => { setSensitivity(e.target.value as Sensitivity); setReviewed(false); }}><option value="uncertain">Ainda não verificada</option><option value="sensitive">Sensível · somente local</option><option value="public">Pública</option></select></label>
            <label className="check"><input type="checkbox" checked={reviewed} disabled={busy || sensitivity !== 'public'} onChange={e => setReviewed(e.target.checked)} />Revisei o pedido e confirmo que é público.</label>
          </> : <p>Para criar imagens, conecte uma conta que gere imagens (OpenAI, Google Gemini, xAI ou OpenRouter) e escolha o modelo. <button type="button" className="link-button" onClick={() => onOpenSettings?.('nuvem')}>Abrir Configurações › Nuvem</button></p>}
        </div>}
        {research && !image && !searchAvailable && <div className="cloud-review"><p>Para pesquisar na web, escolha ChatGPT ou OpenAI em <button type="button" className="link-button" onClick={() => onOpenSettings?.('nuvem')}>Configurações › Nuvem</button>.</p></div>}
        {cloud && !image && (!research || searchAvailable) && <div className="cloud-review">
          {mode === 'per_conversation' && <label className="check"><input type="checkbox" checked={conversation?.cloud_consent ?? false} disabled={busy} onChange={e => void consent(e.target.checked)} />Permitir nuvem nesta conversa</label>}
          {research && <p>Ayla vai pesquisar na web com {provider === 'chatgpt' ? 'seu plano do ChatGPT' : 'a API da OpenAI'}. A pesquisa usa a franquia ou os créditos da conta.</p>}
          {shownReferences.length > 0 && <fieldset className="cloud-references"><legend>Referências que podem ir junto</legend>
            {shownReferences.map(r => <label key={r.id} className="check"><input type="checkbox" checked={!dropped.includes(r.id)} disabled={busy} onChange={e => { setDropped(prev => e.target.checked ? prev.filter(id => id !== r.id) : [...prev, r.id]); setReviewed(false); }} />{r.content}</label>)}
            <small>Só memórias marcadas como públicas e relacionadas ao pedido aparecem aqui.</small>
          </fieldset>}
          {/* Só a confirmação (decisão do usuário, 21/11/2026): marcar a caixa declara o pedido público. */}
          <label className="check"><input type="checkbox" checked={reviewed} disabled={busy || pendingRoute} onChange={e => { setSensitivity(e.target.checked ? 'public' : 'uncertain'); setReviewed(e.target.checked); }} />Confirmo que este pedido não tem dados pessoais e pode ir para a nuvem.</label>
        </div>}
        <form className={listening === 'recording' ? 'composer recording' : 'composer'} onSubmit={e => { e.preventDefault(); if (canSend) void send(); }}>
          {listening !== 'idle' && <p className="composer-status" role="status"><span className="dot" aria-hidden="true" />{listening === 'recording' ? `Ouvindo · ${clock(seconds)}${RECORD_LIMIT_MS / 1000 - seconds <= 30 ? ` · para em ${RECORD_LIMIT_MS / 1000 - seconds} s` : ''}` : 'Transcrevendo…'}</p>}
          {(attachments.length > 0 || adding) && <ul className="composer-files" aria-label="Arquivos anexados">
            {attachments.map(file => <li key={file.id} className="file-chip">{file.kind === 'image' || file.kind === 'gif' ? <AttachmentThumb file={file} /> : <FileIcon size={14} />}<span className="file-name">{file.name}</span><span className="file-meta">{fileLabels[file.kind]} · {formatBytes(file.bytes)}</span><Confirm className="chip-remove" ariaLabel={`Remover ${file.name}`} disabled={busy} question="Tirar este anexo?" confirm="Tirar" onConfirm={() => removeAttachment(file)}><CloseIcon size={14} /></Confirm></li>)}
            {adding && <li className="file-chip" role="status">Guardando…</li>}
          </ul>}
          {bases.length > 0 && <ul className="composer-files" aria-label="Bases nesta mensagem">
            {bases.map(base => <li key={base.id} className="file-chip base-chip"><DatabaseIcon size={14} /><span className="file-name">{base.id.startsWith('project-source:') || base.id.startsWith('imagens:') || base.id.startsWith('academico:') ? 'Fonte' : 'Base'}: {base.name}</span>
              <button type="button" className="chip-remove" aria-label={`Tirar a base ${base.name}`} disabled={busy} onClick={() => onRemoveBase?.(base.id)}><CloseIcon size={14} /></button></li>)}
          </ul>}
          <textarea ref={field} rows={1} aria-label="Mensagem" value={input} readOnly={listening !== 'idle'} disabled={busy}
            placeholder={listening === 'recording' ? 'Fale; o texto aparece aqui…' : image ? 'Descreva a imagem que você quer…' : designing ? 'Descreva o design: tipo (post, story, cartaz…), textos e estilo…' : handing ? 'Diga o que fazer no computador (ex.: põe os wallpapers da Iris na pasta Wallpapers)…' : 'Fale com a Ayla…'}
            onChange={e => { setInput(e.target.value); setReviewed(false); }} onPaste={paste}
            onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); if (canSend) void send(); } }} />
          <div className="composer-actions">
            <button type="button" className="icon-button" aria-label="Anexar arquivos" title="Anexar arquivos (ou arraste e cole)" disabled={busy || cloud || adding || listening !== 'idle'} onClick={() => { void native<string[]>('pick_files').then(paths => attachPaths(paths)).catch(e => setError(errorText(e))); }}><PaperclipIcon /></button>
            <button type="button" className="icon-button" aria-label="Mostrar uma parte da tela" title="Mostrar uma parte da tela à Ayla (fica neste computador)" disabled={busy || cloud || adding || listening !== 'idle'} onClick={() => { void captureScreen().then(file => { if (file) setAttachments(prev => [...prev, file]); }); }}><ScreenIcon /></button>
            <button type="button" className="icon-button" aria-label="Criar design" title="Criar design na Iris (post, story, cartaz…), montado neste computador" aria-pressed={designing} disabled={busy || listening !== 'idle'} onClick={() => { setDesigning(d => !d); setHanding(false); setImageChoice(false); setReviewed(false); }}><LayoutIcon /></button>
            <button type="button" className="icon-button" aria-label="Usar o computador" title="Usar o computador (Hand): a Ayla mostra os passos e só faz depois da sua aprovação" aria-pressed={handing} disabled={busy || listening !== 'idle'} onClick={() => { setHanding(h => !h); setDesigning(false); setImageChoice(false); setReviewed(false); }}><BrandMark name="hand" size={18} /></button>
            {mode !== 'local_only' && <button type="button" className="icon-button" aria-label="Criar imagem" title="Criar imagem (pela nuvem, depois da sua revisão)" aria-pressed={image} disabled={busy || listening !== 'idle'} onClick={() => { setImageChoice(!image); setDesigning(false); setHanding(false); setReviewed(false); }}><ImageIcon /></button>}
            <button type="button" className="icon-button" aria-label="Ajustes desta resposta" aria-expanded={options} aria-controls="response-options" onClick={() => setOptions(o => !o)}><SlidersIcon /></button>
            <span className="spacer" />
            {canHear && !cloud && <button type="button" className={listening === 'recording' ? 'round-button accent' : 'round-button'} aria-label={listening === 'recording' ? 'Terminar de falar' : 'Falar com a Ayla'} aria-pressed={listening === 'recording'} disabled={busy || listening === 'transcribing'} onClick={() => void listen_()}>{listening === 'recording' ? <StopIcon /> : <MicIcon />}</button>}
            {busy
              ? <button type="button" className="round-button" aria-label="Interromper resposta" onClick={() => { if (active.current) void native('cancel_job', { requestId: active.current }); }}><StopIcon /></button>
              : <button type="submit" className="round-button accent" aria-label="Enviar" disabled={!canSend}><ArrowUpIcon size={18} /></button>}
          </div>
        </form>
        <p className="composer-note">{image ? 'A imagem será criada na nuvem depois da sua revisão e ficará guardada nesta conversa.' : handing ? 'Hand: a Ayla mostra o que vai fazer no computador e só faz depois que você aprovar. Fica neste computador.' : designing ? 'A Ayla monta o design neste computador e guarda na Iris › Designs. Para pedidos públicos, pode buscar fotos em bancos livres (só os termos saem).' : destination === 'auto' ? (provider === 'chatgpt' ? <>Automático · Plano do ChatGPT · <button type="button" className="link-button" onClick={() => void native('open_link', { url: 'https://chatgpt.com/settings/usage' }).catch(e => setError(errorText(e)))}>Gerenciar uso</button></> : 'Pedidos públicos podem usar a nuvem. Dados pessoais ficam neste computador.') : cloud ? (snapshot.preferences.cloud_provider === 'chatgpt' ? <>Usando o seu plano do ChatGPT, depois da sua revisão · <button type="button" className="link-button" onClick={() => void native('open_link', { url: 'https://chatgpt.com/settings/usage' }).catch(e => setError(errorText(e)))}>Gerenciar uso</button></> : 'Esta resposta usará a nuvem depois da sua revisão.') : organLocal ? 'A Ayla atende este pedido neste computador.' : 'Respostas geradas neste computador. Nada vai para a nuvem sem a sua revisão.'}</p>
      </div>
    </main>
  </>;
}

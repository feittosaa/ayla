import { useEffect, useLayoutEffect, useRef, useState, type ClipboardEvent } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { native, nativeTask, taskId, errorText, labels, formatBytes, supports, savePreferences, fileLabels, type Capability, type Conversation, type Fact, type FileInfo, type Message, type Project, type Sensitivity, type Snapshot, type ChatRequest, type Effort, type Reference } from '../services/Desktop';
import { base64, recordWav, type Recording } from '../services/Audio';
import { asksForImage } from '../services/Intent';
import { ArrowUpIcon, CloseIcon, CopyIcon, FileIcon, ImageIcon, MicIcon, PaperclipIcon, PencilIcon, SidebarIcon, SlidersIcon, StopIcon } from './Icons';
import Markdown from './Markdown';
import FilePreview from './FilePreview';
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
  /** Abre uma seção das Configurações (ex.: "nuvem"). */
  onOpenSettings?: (section: string) => void;
};
const clock = (seconds: number) => `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;

export default function Chat({ snapshot, refresh, project, conversation, onConversation, onBusy, onShowSidebar, active: visible, onOpenSettings }: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState(''); const [effort, setEffort] = useState<Effort>('');
  // Neste computador, o esforço escolhe entre resposta direta (leve) e raciocínio (alto).
  const kind: Capability | 'auto' = effort === 'low' ? 'text' : effort === 'high' ? 'reasoning' : 'auto';
  const [cloudModel, setCloudModel] = useState(''); const [cloudModels, setCloudModels] = useState<{ id: string; name: string }[]>([]);
  const [model, setModel] = useState(''); const [attachments, setAttachments] = useState<FileInfo[]>([]);
  const [adding, setAdding] = useState(false); const [dragging, setDragging] = useState(false); const [status, setStatus] = useState('');
  /** Escolha explícita de criar imagem; `null` = seguir a frase do pedido. */
  const [imageChoice, setImageChoice] = useState<boolean | null>(null);
  const [titleDraft, setTitleDraft] = useState<string | null>(null);
  const [source, setSource] = useState<'local' | 'cloud'>('local');
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
  const organLocal = localWords.some(word => input.toLowerCase().includes(word));
  const provider = snapshot.preferences.cloud_provider;
  // Modelos da conta conectada, listados quando a resposta vai para a nuvem.
  useEffect(() => {
    if (!options || !provider || !(source === 'cloud' || mode === 'cloud_only')) return;
    let alive = true;
    void native<{ id: string; name: string }[]>('cloud_models', { provider }).then(list => { if (alive) setCloudModels(list); }).catch(() => { if (alive) setCloudModels([]); });
    return () => { alive = false; };
  }, [options, provider, source, mode]);
  const cloud = !organLocal && (source === 'cloud' || mode === 'cloud_only');
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
  const image = (imageChoice ?? asksImage) && mode !== 'local_only';
  // Referências: memórias públicas relevantes, listadas na revisão; só as marcadas vão junto.
  const [references, setReferences] = useState<Reference[]>([]); const [dropped, setDropped] = useState<string[]>([]);
  const reviewing = cloud && !image && input.trim().length >= 3;
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
    : !busy && !adding && listening === 'idle' && Boolean(input.trim() || attachments.length) && (!cloud || reviewed);
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

  // Abre a conversa escolhida na barra do espaço (ou limpa a tela para uma nova).
  useEffect(() => {
    if (created.current === conversationId) { created.current = null; return; }
    setError(''); setReviewed(false); setRouting(''); follow.current = true;
    if (!conversationId) { setMessages([]); return; }
    let current = true;
    native<Message[]>('conversation_messages', { id: conversationId }).then(items => { if (current) setMessages(items); }).catch(e => { if (current) setError(errorText(e)); });
    return () => { current = false; };
  }, [conversationId]);

  useEffect(() => { setReviewed(false); }, [snapshot.system_prompt]);

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
  const attachLatest = useRef(attachPaths);
  useEffect(() => { attachLatest.current = attachPaths; });
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
      setMessages(prev => prev.map((m, i) => i === prev.length - 1 ? { role: 'assistant', content: reply.content, complete: true, attachments: reply.attachments } : m));
    } catch (e) {
      // Nada foi salvo: devolve o pedido para a caixa de mensagem.
      setError(errorText(e)); setMessages(prev => prev.slice(0, -2)); setInput(prompt); setImageChoice(true);
    } finally { active.current = null; setBusy(false); setReviewed(false); setStatus(''); await refresh(); }
  }
  async function send(spokenText?: string) {
    if (image && spokenText === undefined) { await createImage(); return; }
    const typed = (spokenText ?? input).trim();
    const text = typed || (attachments.length ? (attachments.length === 1 ? 'Veja o arquivo anexado.' : 'Veja os arquivos anexados.') : '');
    if (!text || active.current || adding) return;
    const id = taskId(); active.current = id; setBusy(true); setError(''); follow.current = true;
    let current = conversation;
    if (!current) {
      try { current = await native<Conversation>('create_conversation', { project: project?.id ?? null }); created.current = current.id; onConversation(current); }
      catch (e) { setError(errorText(e)); active.current = null; setBusy(false); return; }
    }
    const message = text; if (spokenText === undefined) setInput('');
    const sent = attachments; setAttachments([]);
    setMessages(prev => [...prev, { role: 'user', content: message, complete: true, attachments: sent }, { role: 'assistant', content: '', complete: false }]);
    const request: ChatRequest = { id, conversation: current.id, message, personality_preview: cloud ? snapshot.system_prompt : null, capability: kind === 'auto' ? null : kind, model: model || null, attachments: sent.map(file => file.id), sensitivity, reviewed, request_cloud: source === 'cloud', cloud_model: cloud ? cloudModel || null : null, effort: cloud ? effort || null : null, references: cloud ? kept : [] };
    try {
      await nativeTask<void>('chat', { request }, event => {
        if (event.type === 'routing') setRouting(event.text || '');
        if (event.type === 'suggestion') setError(event.text || 'A Ayla sugeriu revisar o pedido para cloud.');
        if (event.type === 'memory' && event.facts?.length) setLearned(prev => [...prev.filter(f => !event.facts!.some(n => n.id === f.id)), ...event.facts!]);
        if (event.type === 'memory_error') setError(`Não consegui atualizar a memória: ${event.text}`);
        if (event.type === 'status') setStatus(event.text || '');
        if (event.type === 'sources' && event.files?.length) setMessages(prev => { const next = [...prev]; const last = next[next.length - 1]; const known = last.sources || []; next[next.length - 1] = { ...last, sources: [...known, ...event.files!.filter(f => !known.some(k => k.id === f.id))] }; return next; });
        if (event.type === 'token' && event.text) setStatus('');
        if (event.type === 'token' && event.text) setMessages(prev => { const next = [...prev]; const last = next[next.length - 1]; next[next.length - 1] = { ...last, content: last.content + event.text }; return next; });
      });
      setMessages(prev => prev.map((m, index) => index === prev.length - 1 ? { ...m, complete: true } : m));
    } catch (e) { setError(errorText(e)); }
    finally { active.current = null; setBusy(false); setReviewed(false); setStatus(''); await refresh(); }
  }
  async function listen_() {
    if (listening === 'recording') { recorder.current?.stop(); return; }
    if (listening !== 'idle' || busy) return;
    setError('');
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
          .then(result => { if (result.text && recorder.current === session) setInput(result.text); })
          .catch(() => {})
          .finally(() => { previewing = false; });
      }, 1500);
      const audio = await session.result.finally(() => clearInterval(preview)); recorder.current = null;
      setListening('transcribing');
      const id = taskId();
      const result = await native<{ text?: string }>('voice_task', { request: { id, text: '', sensitivity: 'sensitive', reviewed: false, request_cloud: false, cloud_model: null }, audio });
      setListening('idle');
      const heard = (result.text || '').trim();
      setInput('');
      if (heard) await send(heard); else setError('Não entendi o áudio. Tente falar de novo.');
    } catch (e) { recorder.current = null; setListening('idle'); setInput(''); setError(errorText(e)); }
  }
  async function consent(allowed: boolean) {
    if (!conversation) return;
    try { await native('conversation_consent', { id: conversation.id, allowed }); onConversation({ ...conversation, cloud_consent: allowed }); }
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
  async function copy(text: string, index: number) {
    try { await navigator.clipboard.writeText(text); setCopied(index); setTimeout(() => setCopied(c => c === index ? null : c), 1500); }
    catch { setError('Não consegui copiar a resposta.'); }
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
        <span className="chip"><span className="dot" aria-hidden="true" />{cloud ? 'Nuvem' : 'Local'}</span>
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
            ? <div key={i} className="message user">{m.attachments?.length ? <span className="message-files">{m.attachments.map(file => file.kind === 'image' || file.kind === 'gif' ? <FilePreview key={file.id} file={file} /> : <span key={file.id} className="file-chip"><FileIcon size={14} />{file.name}</span>)}</span> : null}{m.content}</div>
            : <div key={i} className="message assistant">
                <span className="orb small" aria-hidden="true" />
                <div className="assistant-body">
                  {m.content ? <Markdown text={m.content} /> : busy && i === messages.length - 1 ? <p className="thinking" role="status">{status || 'Pensando…'}</p> : null}
                  {m.attachments?.map(file => <FilePreview key={file.id} file={file} />)}
                  {m.sources?.length ? <p className="message-sources"><span>Fontes</span>{m.sources.map(source => <span key={source.id} className="file-chip"><FileIcon size={14} />{source.name}{source.date ? ` · ${source.date}` : ''}{source.pages.length ? ` · p. ${source.pages.join(', ')}` : ''}</span>)}</p> : null}
                  {!m.complete && m.content && !busy && <small className="incomplete">Resposta incompleta</small>}
                  {m.complete && m.content && <div className="message-actions"><button className="ghost-button" onClick={() => void copy(m.content, i)}><CopyIcon size={16} />{copied === i ? 'Copiado' : 'Copiar'}</button></div>}
                </div>
              </div>)}
          {learned.map(fact => <div key={fact.id} className="memory-note"><span>Ayla lembrou: {fact.content}</span><button className="link-button" onClick={() => void forget(fact)}>Desfazer</button></div>)}
        </div>
      </div>

      <div className="composer-area">
        {error && <p className="chat-error" role="alert">{error}</p>}
        {options && <div className="response-options" id="response-options" role="group" aria-label="Ajustes desta resposta">
          {routing && <p role="status">Última resposta: {routing}</p>}
          <label>Onde responder<select value={mode === 'cloud_only' ? 'cloud' : source} disabled={busy || mode === 'local_only' || mode === 'cloud_only'} onChange={e => { setSource(e.target.value as 'local' | 'cloud'); setReviewed(false); setEffort(''); }}><option value="local">Neste computador</option><option value="cloud">Na nuvem{provider ? '' : ' · conecte uma conta'}</option></select></label>
          {cloud
            ? <label>Modelo da nuvem<select value={cloudModel} disabled={busy || !provider} onChange={e => setCloudModel(e.target.value)}><option value="">{snapshot.preferences.cloud_model ? `Padrão · ${snapshot.preferences.cloud_model}` : 'Escolha em Configurações › Nuvem'}</option>{cloudModels.map(m => <option key={m.id} value={m.id}>{m.name}</option>)}</select></label>
            : <label>Modelo<select value={model} disabled={busy} onChange={e => setModel(e.target.value)}><option value="">Escolha automática</option>{snapshot.models.filter(m => visual ? supports(m, 'vision') : kind === 'auto' ? (['text', 'reasoning', 'vision'] as Capability[]).some(k => supports(m, k)) : supports(m, kind)).map(m => <option key={m.id} value={m.id}>{m.name}</option>)}</select></label>}
          <label>Esforço<select value={effort} disabled={busy} onChange={e => { setEffort(e.target.value as Effort); if (!cloud) setModel(''); }}>
            <option value="">Automático</option><option value="low">{cloud ? 'Leve' : 'Leve · resposta direta'}</option><option value="medium">Médio</option><option value="high">{cloud ? 'Alto' : 'Alto · raciocínio'}</option>
          </select></label>
          {cloud && effort && <p className="row-hint">O esforço vale para modelos que raciocinam; outros podem recusar o pedido.</p>}
          {mode === 'local_only' && <p className="row-hint">Nuvem desligada. <button type="button" className="link-button" onClick={() => onOpenSettings?.('nuvem')}>Ativar em Configurações › Nuvem</button></p>}
          {mode === 'per_conversation' && conversation && <label className="check"><input type="checkbox" checked={conversation.cloud_consent} disabled={busy} onChange={e => void consent(e.target.checked)} />Permitir nuvem nesta conversa, respeitando a privacidade de cada pedido</label>}
        </div>}
        {image && <div className="cloud-review" aria-label="Revisão do pedido de imagem">
          {imageProvider && snapshot.preferences.image_model ? <>
            <b>Pedido da imagem que será enviado</b><pre>{input || 'Descreva a imagem.'}</pre>
            <p>Só este texto vai para a plataforma de imagens ({snapshot.preferences.image_model}). Histórico, memórias e anexos ficam neste computador.</p>
            <label>Privacidade do pedido<select value={sensitivity} disabled={busy} onChange={e => { setSensitivity(e.target.value as Sensitivity); setReviewed(false); }}><option value="uncertain">Ainda não verificada</option><option value="sensitive">Sensível · somente local</option><option value="public">Pública</option></select></label>
            <label className="check"><input type="checkbox" checked={reviewed} disabled={busy || sensitivity !== 'public'} onChange={e => setReviewed(e.target.checked)} />Revisei o pedido e confirmo que é público.</label>
          </> : <p>Para criar imagens, conecte uma conta que gere imagens (OpenAI, Google Gemini, xAI ou OpenRouter) e escolha o modelo. <button type="button" className="link-button" onClick={() => onOpenSettings?.('nuvem')}>Abrir Configurações › Nuvem</button></p>}
        </div>}
        {cloud && !image && <div className="cloud-review">
          <label>Privacidade do pedido<select value={sensitivity} disabled={busy} onChange={e => { setSensitivity(e.target.value as Sensitivity); setReviewed(false); }}><option value="uncertain">Ainda não verificada</option><option value="sensitive">Sensível · somente local</option><option value="public">Pública</option></select></label>
          <b>Personalidade que será enviada</b><pre>{snapshot.system_prompt}</pre><b>Seu pedido</b><pre>{input || 'Digite o pedido para revisar.'}</pre>
          {shownReferences.length > 0 && <fieldset className="cloud-references"><legend>Referências que podem ir junto</legend>
            {shownReferences.map(r => <label key={r.id} className="check"><input type="checkbox" checked={!dropped.includes(r.id)} disabled={busy} onChange={e => { setDropped(prev => e.target.checked ? prev.filter(id => id !== r.id) : [...prev, r.id]); setReviewed(false); }} />{r.content}</label>)}
            <small>Só memórias marcadas como públicas e relacionadas ao pedido aparecem aqui.</small>
          </fieldset>}
          <p>{kept.length ? 'Somente a personalidade, este pedido e as referências marcadas acima.' : 'Somente a personalidade acima e este pedido.'} Histórico, outras memórias e anexos ficam neste computador.</p>
          <label className="check"><input type="checkbox" checked={reviewed} disabled={busy || sensitivity !== 'public'} onChange={e => setReviewed(e.target.checked)} />Revisei a personalidade e o pedido e confirmo que são públicos.</label>
        </div>}
        <form className={listening === 'recording' ? 'composer recording' : 'composer'} onSubmit={e => { e.preventDefault(); if (canSend) void send(); }}>
          {listening !== 'idle' && <p className="composer-status" role="status"><span className="dot" aria-hidden="true" />{listening === 'recording' ? `Ouvindo · ${clock(seconds)}` : 'Transcrevendo…'}</p>}
          {(attachments.length > 0 || adding) && <ul className="composer-files" aria-label="Arquivos anexados">
            {attachments.map(file => <li key={file.id} className="file-chip"><FileIcon size={14} /><span className="file-name">{file.name}</span><span className="file-meta">{fileLabels[file.kind]} · {formatBytes(file.bytes)}</span><button type="button" className="chip-remove" aria-label={`Remover ${file.name}`} disabled={busy} onClick={() => removeAttachment(file)}><CloseIcon size={14} /></button></li>)}
            {adding && <li className="file-chip" role="status">Guardando…</li>}
          </ul>}
          <textarea ref={field} rows={1} aria-label="Mensagem" value={input} readOnly={listening !== 'idle'} disabled={busy}
            placeholder={listening === 'recording' ? 'Fale; o texto aparece aqui…' : image ? 'Descreva a imagem que você quer…' : 'Fale com a Ayla…'}
            onChange={e => { setInput(e.target.value); setReviewed(false); }} onPaste={paste}
            onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); if (canSend) void send(); } }} />
          <div className="composer-actions">
            <button type="button" className="icon-button" aria-label="Anexar arquivos" title="Anexar arquivos (ou arraste e cole)" disabled={busy || cloud || adding || listening !== 'idle'} onClick={() => { void native<string[]>('pick_files').then(paths => attachPaths(paths)).catch(e => setError(errorText(e))); }}><PaperclipIcon /></button>
            {mode !== 'local_only' && <button type="button" className="icon-button" aria-label="Criar imagem" title="Criar imagem (pela nuvem, depois da sua revisão)" aria-pressed={image} disabled={busy || listening !== 'idle'} onClick={() => { setImageChoice(!image); setReviewed(false); }}><ImageIcon /></button>}
            <button type="button" className="icon-button" aria-label="Ajustes desta resposta" aria-expanded={options} aria-controls="response-options" onClick={() => setOptions(o => !o)}><SlidersIcon /></button>
            <span className="spacer" />
            {canHear && !cloud && <button type="button" className={listening === 'recording' ? 'round-button accent' : 'round-button'} aria-label={listening === 'recording' ? 'Terminar de falar' : 'Falar com a Ayla'} aria-pressed={listening === 'recording'} disabled={busy || listening === 'transcribing'} onClick={() => void listen_()}>{listening === 'recording' ? <StopIcon /> : <MicIcon />}</button>}
            {busy
              ? <button type="button" className="round-button" aria-label="Interromper resposta" onClick={() => { if (active.current) void native('cancel_job', { requestId: active.current }); }}><StopIcon /></button>
              : <button type="submit" className="round-button accent" aria-label="Enviar" disabled={!canSend}><ArrowUpIcon size={18} /></button>}
          </div>
        </form>
        <p className="composer-note">{image ? 'A imagem será criada na nuvem depois da sua revisão e ficará guardada nesta conversa.' : cloud ? (snapshot.preferences.cloud_provider === 'chatgpt' ? <>Usando o seu plano do ChatGPT, depois da sua revisão · <button type="button" className="link-button" onClick={() => void native('open_link', { url: 'https://chatgpt.com/settings/usage' }).catch(e => setError(errorText(e)))}>Gerenciar uso</button></> : 'Esta resposta usará a nuvem depois da sua revisão.') : organLocal ? 'Pergunta sobre finanças: a Ayla consulta a Moneta e responde neste computador.' : 'Respostas geradas neste computador. Nada vai para a nuvem sem a sua revisão.'}</p>
      </div>
    </main>
  </>;
}

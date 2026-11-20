import { useEffect, useLayoutEffect, useRef, useState, type ClipboardEvent } from 'react';
import { listen } from '@tauri-apps/api/event';
import { native, nativeTask, taskId, errorText, labels, formatBytes, supports, type Capability, type Conversation, type Fact, type Message, type Sensitivity, type Snapshot, type ChatRequest } from '../services/Desktop';
import { base64, recordWav, type Recording } from '../services/Audio';
import { ArrowUpIcon, CloudIcon, ComputerIcon, CopyIcon, MicIcon, PaperclipIcon, PlusIcon, SettingsIcon, SidebarIcon, SlidersIcon, StopIcon } from './Icons';
import './Chat.css';

type Props = { snapshot: Snapshot; refresh: () => Promise<void>; onSettings: () => void };
const clock = (seconds: number) => `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;

export default function Chat({ snapshot, refresh, onSettings }: Props) {
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [conversation, setConversation] = useState<Conversation | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState(''); const [kind, setKind] = useState<Capability | 'auto'>('auto');
  const [model, setModel] = useState(''); const [image, setImage] = useState<string | null>(null);
  const [source, setSource] = useState<'local' | 'cloud'>('local');
  const [sensitivity, setSensitivity] = useState<Sensitivity>('uncertain');
  const [reviewed, setReviewed] = useState(false); const [error, setError] = useState('');
  const [routing, setRouting] = useState('');
  const [learned, setLearned] = useState<Fact[]>([]);
  const [busy, setBusy] = useState(false); const active = useRef<string | null>(null);
  const [listening, setListening] = useState<'idle' | 'recording' | 'transcribing'>('idle');
  const [seconds, setSeconds] = useState(0);
  const [options, setOptions] = useState(false);
  const [sidebar, setSidebar] = useState(() => window.innerWidth >= 760);
  const [copied, setCopied] = useState<number | null>(null);
  const recorder = useRef<Recording | null>(null);
  const thread = useRef<HTMLDivElement | null>(null); const field = useRef<HTMLTextAreaElement | null>(null);
  const follow = useRef(true);
  const mode = snapshot.preferences.usage_mode;
  const cloud = source === 'cloud' || mode === 'cloud_only';
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
  const canSend = !busy && listening === 'idle' && Boolean(input.trim()) && (!cloud || reviewed);
  const where = mode === 'local_only' ? 'Tudo roda neste computador' : mode === 'cloud_only' ? 'Respostas pela nuvem, com sua revisão' : 'Neste computador; nuvem só com sua revisão';

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

  useEffect(() => {
    native<Conversation[]>('conversations').then(setConversations).catch(e => setError(errorText(e)));
    return () => { if (active.current) void native('cancel_job', { requestId: active.current }); recorder.current?.stop(); };
  }, []);

  useEffect(() => { setReviewed(false); }, [snapshot.system_prompt]);

  // Acompanha a resposta enquanto a pessoa estiver no fim da conversa.
  useLayoutEffect(() => { const el = thread.current; if (el && follow.current) el.scrollTop = el.scrollHeight; }, [messages, learned]);
  // Campo cresce com o texto, até um limite.
  useLayoutEffect(() => { const el = field.current; if (!el) return; el.style.height = 'auto'; el.style.height = `${Math.min(el.scrollHeight, 200)}px`; }, [input]);
  useEffect(() => {
    if (listening !== 'recording') return;
    setSeconds(0); const timer = setInterval(() => setSeconds(s => s + 1), 1000);
    return () => clearInterval(timer);
  }, [listening]);

  async function open(item: Conversation) {
    if (active.current) return;
    try { setMessages(await native<Message[]>('conversation_messages', { id: item.id })); setConversation(item); setError(''); setReviewed(false); follow.current = true; }
    catch (e) { setError(errorText(e)); }
  }
  // A conversa só é criada na primeira mensagem; "Nova conversa" apenas limpa a tela.
  function newConversation() {
    if (active.current) return;
    setConversation(null); setMessages([]); setError(''); setReviewed(false); setRouting('');
    field.current?.focus();
  }
  async function send(spokenText?: string) {
    const text = (spokenText ?? input).trim();
    if (!text || active.current) return;
    const id = taskId(); active.current = id; setBusy(true); setError(''); follow.current = true;
    let current = conversation;
    if (!current) {
      try { current = await native<Conversation>('create_conversation'); setConversation(current); setConversations(prev => [current!, ...prev]); }
      catch (e) { setError(errorText(e)); active.current = null; setBusy(false); return; }
    }
    const message = text; if (spokenText === undefined) setInput('');
    setMessages(prev => [...prev, { role: 'user', content: message, complete: true }, { role: 'assistant', content: '', complete: false }]);
    const request: ChatRequest = { id, conversation: current.id, message, personality_preview: cloud ? snapshot.system_prompt : null, capability: kind === 'auto' ? null : kind, model: model || null, image, sensitivity, reviewed, request_cloud: source === 'cloud' };
    try {
      await nativeTask<void>('chat', { request }, event => {
        if (event.type === 'routing') setRouting(event.text || '');
        if (event.type === 'suggestion') setError(event.text || 'A Ayla sugeriu revisar o pedido para cloud.');
        if (event.type === 'memory' && event.facts?.length) setLearned(prev => [...prev.filter(f => !event.facts!.some(n => n.id === f.id)), ...event.facts!]);
        if (event.type === 'memory_error') setError(`Não consegui atualizar a memória: ${event.text}`);
        if (event.type === 'token' && event.text) setMessages(prev => { const next = [...prev]; const last = next[next.length - 1]; next[next.length - 1] = { ...last, content: last.content + event.text }; return next; });
      });
      setMessages(prev => prev.map((m, index) => index === prev.length - 1 ? { ...m, complete: true } : m));
    } catch (e) { setError(errorText(e)); }
    finally { active.current = null; setBusy(false); setReviewed(false); setImage(null); await refresh(); }
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
    try { await native('conversation_consent', { id: conversation.id, allowed }); setConversation({ ...conversation, cloud_consent: allowed }); setConversations(prev => prev.map(c => c.id === conversation.id ? { ...c, cloud_consent: allowed } : c)); }
    catch (e) { setError(errorText(e)); }
  }
  // Imagem colada (Ctrl+V): vira um anexo local, como uma imagem escolhida.
  function paste(event: ClipboardEvent<HTMLTextAreaElement>) {
    const file = Array.from(event.clipboardData.items).find(item => item.kind === 'file' && item.type.startsWith('image/'))?.getAsFile();
    if (!file) return;
    event.preventDefault();
    if (cloud) { setError('Imagens ficam neste computador. Responda localmente para usar a imagem.'); return; }
    setError('');
    void file.arrayBuffer()
      .then(buffer => native<string>('paste_image', { data: base64(new Uint8Array(buffer)) }))
      .then(setImage)
      .catch(e => setError(errorText(e)));
  }
  async function copy(text: string, index: number) {
    try { await navigator.clipboard.writeText(text); setCopied(index); setTimeout(() => setCopied(c => c === index ? null : c), 1500); }
    catch { setError('Não consegui copiar a resposta.'); }
  }

  return <div className="chat-layout">
    {sidebar && <aside className="sidebar" aria-label="Conversas">
      <div className="brand"><span className="orb" aria-hidden="true" /><span className="brand-name">Ayla</span>
        <button className="icon-button" aria-label="Recolher a lista de conversas" onClick={() => setSidebar(false)}><SidebarIcon /></button></div>
      <button className="new-chat" disabled={busy} onClick={newConversation}><PlusIcon size={18} />Nova conversa</button>
      <nav className="conversation-list" aria-label="Conversas recentes">
        {conversations.length > 0 && <p className="sidebar-label">Recentes</p>}
        {conversations.map(item => <button key={item.id} className="conversation-item" aria-current={item.id === conversationId ? 'page' : undefined} disabled={busy && item.id !== conversationId} onClick={() => void open(item)}>{item.title}</button>)}
      </nav>
      <div className="sidebar-footer">
        <p className="where">{mode === 'cloud_only' ? <CloudIcon size={18} /> : <ComputerIcon size={18} />}{where}</p>
        <button className="sidebar-link" onClick={onSettings}><SettingsIcon size={18} />Configurações</button>
      </div>
    </aside>}

    <main className="chat-main">
      <header className="chat-header">
        {!sidebar && <button className="icon-button" aria-label="Mostrar a lista de conversas" onClick={() => setSidebar(true)}><SidebarIcon /></button>}
        <h1>{conversation?.title || 'Nova conversa'}</h1>
        <span className="chip"><span className="dot" aria-hidden="true" />{cloud ? 'Nuvem' : 'Local'}</span>
        {!sidebar && <button className="icon-button" aria-label="Configurações" onClick={onSettings}><SettingsIcon /></button>}
      </header>

      <div className="messages" ref={thread} aria-live="polite" onScroll={e => { const el = e.currentTarget; follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80; }}>
        <div className="thread">
          {!messages.length && !needsSetup && !setup.active && <div className="empty-state"><span className="orb large" aria-hidden="true" /><p>O que vamos fazer hoje?</p></div>}
          {!messages.length && (needsSetup || setup.active) && <section className="setup-card" aria-label="Preparar a Ayla">
            <h2>{setup.active ? 'Preparando a Ayla neste computador' : 'Vamos preparar a Ayla neste computador'}</h2>
            {setup.active ? <><p>Baixando os modelos padrão para a Ayla funcionar sem internet. Você pode usar outras telas enquanto isso.</p>
              <div role="status" className="setup-progress"><progress max={setup.total || 1} value={setup.received || 0} aria-label="Progresso do preparo" /><span>{setup.label} · {formatBytes(setup.received || 0)} de {formatBytes(setup.total || 0)}</span><button onClick={() => void native('cancel_job', { requestId: 'auto-setup' }).then(refresh)}>Interromper</button></div></>
              : suggested ? <><p>{setup.error ? `O preparo não terminou: ${setup.error}` : setup.cancelled ? 'Preparo interrompido.' : 'A Ayla precisa de um modelo de IA para conversar sem internet.'} Recomendado para o seu computador: <b>{suggested.name}</b>.</p><button className="primary-action" onClick={start}>Preparar a Ayla · {formatBytes(firstSetup)}</button></>
              : <p>Os modelos recomendados não cabem na memória livre agora (disponível: {formatBytes(snapshot.memory_budget_bytes)}). Feche alguns programas, escolha uma preferência de uso maior em Configurações ou importe um modelo menor nas opções avançadas.</p>}
          </section>}
          {!messages.length && offerMemory && <section className="setup-card" aria-label="Ativar memória"><h2>Quer que a Ayla lembre de você?</h2><p>Com a memória ativada, a Ayla guarda neste computador o que for importante nas conversas, como preferências, pessoas e projetos. Você vê e apaga tudo em Configurações.</p><button className="primary-action" onClick={start}>Ativar memória · {formatBytes(memoryEntry.download_bytes)}</button></section>}

          {messages.map((m, i) => m.role === 'user'
            ? <div key={i} className="message user">{m.content}</div>
            : <div key={i} className="message assistant">
                <span className="orb small" aria-hidden="true" />
                <div className="assistant-body">
                  {m.content ? <div className="assistant-text">{m.content}</div> : busy && i === messages.length - 1 ? <p className="thinking">Pensando…</p> : null}
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
          <label>Tipo de pedido<select value={kind} disabled={busy} onChange={e => { setKind(e.target.value as Capability | 'auto'); setModel(''); }}>
            <option value="auto">Automático pelo pedido</option>
            {(['text', 'reasoning', 'vision'] as Capability[]).map(k => <option key={k} value={k}>{labels[k]}</option>)}
          </select></label>
          <label>Modelo<select value={model} disabled={busy} onChange={e => setModel(e.target.value)}><option value="">Escolha automática</option>{snapshot.models.filter(m => image ? supports(m, 'vision') : kind === 'auto' ? (['text', 'reasoning', 'vision'] as Capability[]).some(k => supports(m, k)) : supports(m, kind)).map(m => <option key={m.id} value={m.id}>{m.name}</option>)}</select></label>
          <label>Onde responder<select value={source} disabled={busy || mode === 'local_only' || mode === 'cloud_only'} onChange={e => { setSource(e.target.value as 'local' | 'cloud'); setReviewed(false); }}><option value="local">Neste computador</option><option value="cloud">Na nuvem</option></select></label>
          {mode === 'per_conversation' && conversation && <label className="check"><input type="checkbox" checked={conversation.cloud_consent} disabled={busy} onChange={e => void consent(e.target.checked)} />Permitir nuvem nesta conversa, respeitando a privacidade de cada pedido</label>}
        </div>}
        {cloud && <div className="cloud-review">
          <label>Privacidade do pedido<select value={sensitivity} disabled={busy} onChange={e => { setSensitivity(e.target.value as Sensitivity); setReviewed(false); }}><option value="uncertain">Ainda não verificada</option><option value="sensitive">Sensível · somente local</option><option value="public">Pública</option></select></label>
          <b>Personalidade que será enviada</b><pre>{snapshot.system_prompt}</pre><b>Seu pedido</b><pre>{input || 'Digite o pedido para revisar.'}</pre>
          <p>Somente a personalidade acima e este pedido. Histórico, memórias e anexos não serão enviados.</p>
          <label className="check"><input type="checkbox" checked={reviewed} disabled={busy || sensitivity !== 'public'} onChange={e => setReviewed(e.target.checked)} />Revisei a personalidade e o pedido e confirmo que são públicos.</label>
        </div>}
        <form className={listening === 'recording' ? 'composer recording' : 'composer'} onSubmit={e => { e.preventDefault(); if (canSend) void send(); }}>
          {listening !== 'idle' && <p className="composer-status" role="status"><span className="dot" aria-hidden="true" />{listening === 'recording' ? `Ouvindo · ${clock(seconds)}` : 'Transcrevendo…'}</p>}
          {image && <p className="attachment">Imagem anexada · fica neste computador <button type="button" className="link-button" disabled={busy} onClick={() => setImage(null)}>Remover</button></p>}
          <textarea ref={field} rows={1} aria-label="Mensagem" value={input} readOnly={listening !== 'idle'} disabled={busy}
            placeholder={listening === 'recording' ? 'Fale; o texto aparece aqui…' : 'Fale com a Ayla…'}
            onChange={e => { setInput(e.target.value); setReviewed(false); }} onPaste={paste}
            onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); if (canSend) void send(); } }} />
          <div className="composer-actions">
            <button type="button" className="icon-button" aria-label="Anexar imagem" disabled={busy || cloud || listening !== 'idle'} onClick={() => { void native<string | null>('pick_image').then(path => { if (path) setImage(path); }).catch(e => setError(errorText(e))); }}><PaperclipIcon /></button>
            <button type="button" className="icon-button" aria-label="Ajustes desta resposta" aria-expanded={options} aria-controls="response-options" onClick={() => setOptions(o => !o)}><SlidersIcon /></button>
            <span className="spacer" />
            {canHear && !cloud && <button type="button" className={listening === 'recording' ? 'round-button accent' : 'round-button'} aria-label={listening === 'recording' ? 'Terminar de falar' : 'Falar com a Ayla'} aria-pressed={listening === 'recording'} disabled={busy || listening === 'transcribing'} onClick={() => void listen_()}>{listening === 'recording' ? <StopIcon /> : <MicIcon />}</button>}
            {busy
              ? <button type="button" className="round-button" aria-label="Interromper resposta" onClick={() => { if (active.current) void native('cancel_job', { requestId: active.current }); }}><StopIcon /></button>
              : <button type="submit" className="round-button accent" aria-label="Enviar" disabled={!canSend}><ArrowUpIcon size={18} /></button>}
          </div>
        </form>
        <p className="composer-note">{cloud ? 'Esta resposta usará a nuvem depois da sua revisão.' : 'Respostas geradas neste computador. Nada vai para a nuvem sem a sua revisão.'}</p>
      </div>
    </main>
  </div>;
}

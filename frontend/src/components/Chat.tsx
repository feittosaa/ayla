import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { native, nativeTask, taskId, errorText, labels, formatBytes, supports, type Capability, type Conversation, type Fact, type Message, type Sensitivity, type Snapshot, type ChatRequest } from '../services/Desktop';
import ModelSetup from './ModelSetup';
import './Chat.css';

type Props = { snapshot: Snapshot; refresh: () => Promise<void> };
export default function Chat({ snapshot, refresh }: Props) {
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
  const cloud = source === 'cloud' || snapshot.preferences.usage_mode === 'cloud_only';
  const needsSetup = snapshot.preferences.usage_mode !== 'cloud_only' && !snapshot.models.some(m => supports(m, 'text'));
  const suggested = snapshot.catalog.find(entry => entry.id === snapshot.recommended_model);
  const memoryEntry = snapshot.catalog.find(entry => entry.capabilities.includes('embedding'));
  const offerMemory = !needsSetup && snapshot.preferences.memory_auto && !snapshot.models.some(m => supports(m, 'embedding')) && memoryEntry;
  const conversationId = conversation?.id;

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
    return () => { if (active.current) void native('cancel_job', { requestId: active.current }); };
  }, []);

  useEffect(() => { setReviewed(false); }, [snapshot.system_prompt]);

  async function open(item: Conversation) {
    if (active.current) return;
    try { setMessages(await native<Message[]>('conversation_messages', { id: item.id })); setConversation(item); setError(''); setReviewed(false); }
    catch (e) { setError(errorText(e)); }
  }
  async function newConversation() {
    try { const item = await native<Conversation>('create_conversation'); setConversations(prev => [item, ...prev]); await open(item); }
    catch (e) { setError(errorText(e)); }
  }
  async function send() {
    if (!input.trim() || active.current) return;
    const id = taskId(); active.current = id; setBusy(true); setError('');
    let current = conversation;
    if (!current) {
      try { current = await native<Conversation>('create_conversation'); setConversation(current); setConversations(prev => [current!, ...prev]); }
      catch (e) { setError(errorText(e)); active.current = null; setBusy(false); return; }
    }
    const message = input.trim(); setInput('');
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
  async function consent(allowed: boolean) {
    if (!conversation) return;
    try { await native('conversation_consent', { id: conversation.id, allowed }); setConversation({ ...conversation, cloud_consent: allowed }); setConversations(prev => prev.map(c => c.id === conversation.id ? { ...c, cloud_consent: allowed } : c)); }
    catch (e) { setError(errorText(e)); }
  }

  return <div className="chat-container">
    <div className="chat-toolbar">
      <select aria-label="Conversa" value={conversation?.id || ''} disabled={busy} onChange={e => { const item = conversations.find(c => c.id === e.target.value); if (item) void open(item); }}><option value="">Nova conversa</option>{conversations.map((c, index) => <option key={c.id} value={c.id}>{c.title} · {conversations.length - index}</option>)}</select>
      <button disabled={busy} onClick={() => void newConversation()}>Nova</button>
    </div>
    <div className="messages" aria-live="polite">
      {!conversation && !needsSetup && <p className="empty-state">O que vamos fazer hoje?</p>}
      {!messages.length && needsSetup && <section className="setup-card" aria-label="Preparar a Ayla">
        <h2>Vamos preparar a Ayla neste computador</h2>
        {suggested ? <><p>Para conversar sem internet, a Ayla precisa de um modelo de IA. Recomendado para o seu computador: <b>{suggested.name}</b>. {suggested.summary}</p><ModelSetup entry={suggested} refresh={refresh} action="Preparar a Ayla" /></>
          : <p>Os modelos recomendados não cabem na memória livre agora (disponível: {formatBytes(snapshot.memory_budget_bytes)}). Feche alguns programas, escolha uma preferência de uso maior em Configurações ou importe um modelo menor nas opções avançadas.</p>}
      </section>}
      {messages.map((m, i) => <div key={i} className={`message ${m.role}`}>{m.content}{!m.complete && m.content && !busy && <small>Resposta incompleta</small>}</div>)}
      {busy && <p className="connection-status">Respondendo…</p>}
      {learned.length > 0 && <div className="memory-notes" aria-label="Memórias desta conversa">{learned.map(fact => <p key={fact.id}>Ayla lembrou: {fact.content} <button className="link-button" onClick={() => void forget(fact)}>desfazer</button></p>)}</div>}
      {!messages.length && offerMemory && <section className="setup-card" aria-label="Ativar memória"><h2>Quer que a Ayla lembre de você?</h2><p>Com a memória ativada, a Ayla guarda neste computador o que for importante nas conversas, como preferências, pessoas e projetos. Você vê e apaga tudo em Configurações.</p><ModelSetup entry={memoryEntry} refresh={refresh} action="Ativar memória" /></section>}
    </div>
    {error && <p className="chat-error" role="alert">{error}</p>}
    <div className="chat-options">
      <details className="advanced-chat"><summary>Personalizar esta resposta</summary>
      {routing && <p role="status">{routing}</p>}
      <select aria-label="Tipo de pedido" value={kind} disabled={busy} onChange={e => { setKind(e.target.value as Capability | 'auto'); setModel(''); }}>
        <option value="auto">Tipo automático pelo pedido</option>
        {(['text', 'reasoning', 'vision'] as Capability[]).map(k => <option key={k} value={k}>{labels[k]}</option>)}
      </select>
      <select aria-label="Modelo local" value={model} disabled={busy} onChange={e => setModel(e.target.value)}><option value="">Seleção automática</option>{snapshot.models.filter(m => image ? supports(m, 'vision') : kind === 'auto' ? (['text', 'reasoning', 'vision'] as Capability[]).some(k => supports(m, k)) : supports(m, kind)).map(m => <option key={m.id} value={m.id}>{m.name}</option>)}</select>
      <select aria-label="Onde responder" value={source} disabled={busy || snapshot.preferences.usage_mode === 'local_only' || snapshot.preferences.usage_mode === 'cloud_only'} onChange={e => { setSource(e.target.value as 'local' | 'cloud'); setReviewed(false); }}><option value="local">Neste computador</option><option value="cloud">Na nuvem</option></select>
      </details>
      <button disabled={busy || cloud} onClick={() => { void native<string | null>('pick_image').then(setImage).catch(e => setError(errorText(e))); }}>Anexar imagem</button>
      {image && <p>Imagem anexada · permanece local <button disabled={busy} onClick={() => setImage(null)}>Remover</button></p>}
      {cloud && <select aria-label="Privacidade do pedido" value={sensitivity} disabled={busy} onChange={e => { setSensitivity(e.target.value as Sensitivity); setReviewed(false); }}><option value="uncertain">Privacidade ainda não verificada</option><option value="sensitive">Conteúdo sensível · somente local</option><option value="public">Conteúdo público</option></select>}
      {snapshot.preferences.usage_mode === 'per_conversation' && conversation && <label><input type="checkbox" checked={conversation.cloud_consent} disabled={busy} onChange={e => void consent(e.target.checked)} />Permitir cloud nesta conversa, respeitando a privacidade de cada pedido</label>}
      {cloud && <div className="cloud-review"><b>Personalidade que será enviada</b><pre>{snapshot.system_prompt}</pre><b>Seu pedido</b><pre>{input || 'Digite o pedido para revisar.'}</pre><p>Somente a personalidade acima e este pedido. Histórico, memórias e anexos não serão enviados.</p><label><input type="checkbox" checked={reviewed} disabled={busy || sensitivity !== 'public'} onChange={e => setReviewed(e.target.checked)} />Revisei a personalidade e o pedido e confirmo que são públicos.</label></div>}
    </div>
    <form className="input-bar" onSubmit={e => { e.preventDefault(); void send(); }}>
      <input aria-label="Mensagem" value={input} onChange={e => { setInput(e.target.value); setReviewed(false); }} placeholder="Fale com a Ayla…" disabled={busy} />
      {busy ? <button type="button" onClick={() => { if (active.current) void native('cancel_job', { requestId: active.current }); }}>Interromper</button> : <button disabled={!input.trim() || (cloud && !reviewed)}>Enviar</button>}
    </form>
  </div>;
}

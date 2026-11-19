import { useCallback, useEffect, useState } from 'react';
import Chat from './components/Chat';
import Models from './components/Models';
import Capabilities from './components/Capabilities';
import MemoryPanel from './components/MemoryPanel';
import { desktopSnapshot, errorText, formatBytes, gib, modeLabels, native, savePreferences, type Preferences, type Profile, type Snapshot, type UsageMode } from './services/Desktop';
import './App.css';

const profiles: { id: Profile; title: string; percent: number; text: string }[] = [
  { id: 'light', title: 'Leve', percent: 25, text: 'Priorizar economia de recursos.' },
  { id: 'moderate', title: 'Moderado', percent: 50, text: 'Equilibrar recursos e capacidade.' },
  { id: 'maximum', title: 'Máximo', percent: 75, text: 'Priorizar capacidade dentro do hardware disponível.' },
];
type Page = 'chat' | 'models' | 'capabilities' | 'settings';
const pages: { id: Page; label: string }[] = [{ id: 'chat', label: 'Conversa' }, { id: 'settings', label: 'Configurações' }];
export default function App() {
  const [page, setPage] = useState<Page>('chat'); const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [draft, setDraft] = useState<Preferences | null>(null); const [error, setError] = useState(''); const [notice, setNotice] = useState(''); const [saving, setSaving] = useState(false);
  const [key, setKey] = useState(''); const [remember, setRemember] = useState(false);
  const refresh = useCallback(async () => {
    try { const value = await desktopSnapshot(); setSnapshot(value); setDraft(prev => prev || value.preferences); }
    catch (e) { setError(errorText(e)); }
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  function update(values: Partial<Preferences>) { setDraft(prev => prev ? { ...prev, ...values } : prev); setNotice(''); }
  async function apply() {
    if (!draft || saving) return; setSaving(true); setNotice(''); setError('');
    try { const value = { ...draft, onboarding_complete: true }; await savePreferences(value); setDraft(value); await refresh(); setNotice('Preferências salvas.'); }
    catch (e) { setError(errorText(e)); } finally { setSaving(false); }
  }
  async function saveKey() {
    setSaving(true); setError('');
    try { await native('cloud_key', { key, remember, endpoint: draft?.cloud_endpoint || '' }); setKey(''); setNotice(remember ? 'Chave guardada no cofre do sistema.' : 'Chave disponível somente nesta sessão.'); }
    catch (e) { setError(errorText(e)); } finally { setSaving(false); }
  }
  const providerSaved = Boolean(draft?.cloud_endpoint) && draft?.cloud_endpoint === snapshot?.preferences.cloud_endpoint;
  const profile = draft?.profile || snapshot?.recommended_profile || 'moderate';
  return <div className="app-shell">
    <header className="app-header"><strong>Ayla</strong><span>Seu espaço pessoal</span></header>
    <nav aria-label="Navegação principal">{pages.map(item => <button key={item.id} aria-current={(page === item.id || (item.id === 'settings' && page !== 'chat')) ? 'page' : undefined} onClick={() => { setPage(item.id); setNotice(''); setError(''); }}>{item.label}</button>)}</nav>
    {!snapshot && <main className="dashboard">{error ? <p role="alert" className="app-error">{error}</p> : <p role="status">Preparando seu espaço…</p>}</main>}
    {snapshot && draft && <>
      {page === 'chat' && <main className="chat-page"><Chat snapshot={snapshot} refresh={refresh} /></main>}
      {page === 'models' && <main className="content-page"><button className="settings-back" onClick={() => setPage('settings')}>← Configurações</button><Models snapshot={snapshot} refresh={refresh} /></main>}
      {page === 'capabilities' && <main className="content-page"><button className="settings-back" onClick={() => setPage('settings')}>← Configurações</button><Capabilities snapshot={snapshot} refresh={refresh} /></main>}
      {page === 'settings' && <main className="dashboard">
        {error && <p role="alert" className="app-error">{error}</p>}
        <section><h1>Configurações</h1><p>A Ayla cuida das escolhas do dia a dia. Personalize apenas se quiser.</p></section>
        <section className="panel"><h2>Perfil Ayla</h2><p>{draft.profile === null && draft.resource_percent === null ? 'Automático: recursos ajustados ao seu computador e modelo escolhido conforme o pedido.' : 'Você está usando ajustes personalizados.'}</p><button disabled={saving} onClick={async () => { setSaving(true); try { const value = { ...draft, profile: null, resource_percent: null, onboarding_complete: true }; await savePreferences(value); setDraft(value); await refresh(); setNotice('Perfil automático ativado.'); } catch (e) { setError(errorText(e)); } finally { setSaving(false); } }}>Usar configuração automática</button>{!snapshot.models.length && <p>A conversa com IA ainda precisa de um modelo. Você pode prepará-lo na tela de conversa, com confirmação do download.</p>}</section>
        <section className="panel" aria-labelledby="profile-title"><h2 id="profile-title">Preferência de uso</h2><p>Perfil sugerido: {profiles.find(p => p.id === snapshot.recommended_profile)?.title}. Você pode ajustar; salvar não baixa nem carrega modelos.</p><fieldset className="profiles"><legend>Perfil de recursos</legend><label className={draft.profile === null && draft.resource_percent === null ? 'selected' : ''}><input type="radio" name="profile" checked={draft.profile === null && draft.resource_percent === null} onChange={() => update({ profile: null, resource_percent: null })} /><span><b>Automático · recomendado</b><small>A Ayla ajusta os recursos ao seu computador.</small></span></label>{profiles.map(item => <label key={item.id} className={draft.profile === item.id ? 'selected' : ''}><input type="radio" name="profile" value={item.id} checked={draft.profile === item.id} onChange={() => update({ profile: item.id, resource_percent: null })} /><span><b>{item.title} · até {item.percent}% da RAM</b><small>{item.text}</small></span></label>)}</fieldset>
          <details><summary>Ajustes de recursos</summary><label>Limite personalizado de RAM (%)<input type="number" min={5} max={90} value={draft.resource_percent ?? profiles.find(p => p.id === profile)?.percent ?? 50} onChange={e => update({ resource_percent: Number(e.target.value) })} /></label><label>Contexto máximo do modelo (tokens)<input type="number" min={512} max={32768} step={512} value={draft.context_tokens} onChange={e => update({ context_tokens: Number(e.target.value) })} /></label><label>Liberar modelo após inatividade (segundos)<input type="number" min={0} max={3600} value={draft.idle_seconds} onChange={e => update({ idle_seconds: Number(e.target.value) })} /></label><p>O orçamento limita a estimativa de carregamento; consumo real varia entre arquiteturas. Modelos incompatíveis produzem um erro sem carregar outro automaticamente.</p></details>
          <button className="primary-action" disabled={saving} onClick={() => void apply()}>{saving ? 'Salvando…' : 'Salvar preferências'}</button>
        </section>
        <section className="panel"><h2>Personalidade</h2><p>Como você quer que a Ayla converse? Vale mudar o tom, o humor e o nível de detalhe. As mudanças valem para as próximas respostas.</p><label htmlFor="personality">Jeito de responder</label><textarea id="personality" rows={5} maxLength={4000} value={draft.personality} placeholder="Ex.: seja acolhedora, direta e use humor leve. Explique com exemplos simples." onChange={e => update({ personality: e.target.value })} /><p>{draft.personality.trim() ? 'Personalidade personalizada.' : 'Usando a personalidade original da Ayla.'} As regras de privacidade continuam valendo.</p><button className="primary-action" disabled={saving} onClick={() => void apply()}>Salvar personalidade</button><button disabled={saving} onClick={async () => { setSaving(true); try { const value = { ...snapshot.preferences, personality: '' }; await savePreferences(value); setDraft(prev => prev ? { ...prev, personality: '' } : value); await refresh(); setNotice('Personalidade original restaurada.'); } catch (e) { setError(errorText(e)); } finally { setSaving(false); } }}>Restaurar personalidade original</button></section>
        <MemoryPanel snapshot={snapshot} refresh={refresh} />
        <details className="panel"><summary>Opções avançadas</summary><p>Controles opcionais para personalizar o funcionamento da Ayla.</p><div className="button-row"><button onClick={() => setPage('models')}>Modelos e componentes</button><button onClick={() => setPage('capabilities')}>Ferramentas de voz e memória</button></div>
        <section className="panel" aria-labelledby="hardware-title"><h2 id="hardware-title">Seu computador</h2><div className="hardware-summary"><span><b>{gib(snapshot.hardware.ram_total_bytes)} GB</b> de memória</span><span><b>{snapshot.hardware.logical_cpus}</b> processadores lógicos</span></div><p>{snapshot.hardware.gpus.length ? snapshot.hardware.gpus.map(g => `${g.vendor}${g.vram_total_bytes ? ` · ${gib(g.vram_total_bytes)} GB gráficos detectados` : ' · memória gráfica não identificada'}`).join(' / ') : 'Memória gráfica não identificada. Isso não impede o uso da Ayla.'}</p><p>Orçamento disponível agora: {formatBytes(snapshot.memory_budget_bytes)}, com reserva para o sistema.</p><button onClick={() => void refresh()}>Atualizar estado</button></section>
        {page === 'settings' && <>
          <section className="panel"><h2>Onde a IA trabalha</h2><label>Modo de uso<select value={draft.usage_mode} onChange={e => update({ usage_mode: e.target.value as UsageMode })}>{Object.entries(modeLabels).map(([mode, label]) => <option key={mode} value={mode}>{label}</option>)}</select></label><p>Conteúdo sensível ou ainda não verificado fica local em todos os modos. A nuvem recebe o pedido e a personalidade, mostrados para revisão e declarados públicos. Sugestões não autorizam envios.</p><label>Endereço do provedor compatível<input type="url" value={draft.cloud_endpoint} onChange={e => update({ cloud_endpoint: e.target.value })} placeholder="https://seu-provedor/v1" /></label><label>Modelo cloud para conversa<input value={draft.cloud_model} onChange={e => update({ cloud_model: e.target.value })} /></label><button className="primary-action" disabled={saving} onClick={() => void apply()}>Salvar modo e provedor</button><label>Chave do provedor<input type="password" autoComplete="off" value={key} onChange={e => setKey(e.target.value)} /></label><label><input type="checkbox" checked={remember} onChange={e => setRemember(e.target.checked)} />Guardar no cofre do sistema</label><button disabled={saving || !key || !providerSaved} onClick={() => void saveKey()}>{remember ? 'Guardar chave no cofre' : 'Usar chave nesta sessão'}</button><button disabled={saving || !providerSaved} onClick={() => { void native('cloud_key', { key: '', remember: false, endpoint: draft.cloud_endpoint }).then(() => setNotice('Chave removida.')).catch(e => setError(errorText(e))); }}>Esquecer chave deste provedor</button></section>
          <section className="panel"><h2>Memória e dados</h2><label>Mensagens recentes no contexto<input type="number" min={2} max={200} value={draft.history_messages} onChange={e => update({ history_messages: Number(e.target.value) })} /></label><label>Limite de caracteres do histórico<input type="number" min={1000} max={200000} value={draft.history_chars} onChange={e => update({ history_chars: Number(e.target.value) })} /></label><p>As conversas são isoladas; respostas interrompidas ficam identificadas e não entram no contexto. Histórico antigo permanece no banco local, sem resumos aleatórios.</p><button disabled={saving} onClick={() => void apply()}>Salvar limites</button><button onClick={() => { void native<number | null>('import_legacy').then(count => { if (count !== null) setNotice(`Histórico importado: ${count} mensagens em uma conversa separada. Fonte original preservada.`); }).catch(e => setError(errorText(e))); }}>Importar cópia do histórico anterior</button><button onClick={() => { void native('backup_data').then(() => setNotice('Backup consistente criado na pasta de dados da Ayla.')).catch(e => setError(errorText(e))); }}>Criar backup</button></section>
          <section className="panel"><h2>Medições locais</h2><p>As últimas tarefas registram tempo até o primeiro trecho, duração e quantidade de trechos. Trechos de streaming não equivalem necessariamente a tokens do modelo.</p>{!snapshot.metrics.length && <p>Ainda sem medições de geração.</p>}{snapshot.metrics.slice(0, 5).map((m, i) => <p key={i}>Primeiro trecho: {m.first_token_ms ?? '—'} ms · duração: {m.duration_ms} ms · {m.chunks} trechos{m.cancelled ? ' · interrompida' : ''}</p>)}</section>
        </>}
        </details>
        {notice && <p role="status">{notice}</p>}
      </main>}
    </>}
  </div>;
}

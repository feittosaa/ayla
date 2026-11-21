import { useCallback, useEffect, useLayoutEffect, useState, type FormEvent } from 'react';
import { DesktopSettings, quickStyle, useDesktopView } from './components/DesktopIntegration';
import Space from './components/Space';
import Models from './components/Models';
import Capabilities from './components/Capabilities';
import MemoryPanel from './components/MemoryPanel';
import CloudSettings from './components/CloudSettings';
import { OrganIcon, OrganInstallDialog, OrganSettings, OrganSpace, useOrgan } from './components/Organ';
import { ChevronLeftIcon, ChevronRightIcon, ExpandIcon, SettingsIcon } from './components/Icons';
import { desktopSnapshot, supports, errorText, formatBytes, gib, native, savePreferences, type Preferences, type Profile, type Project, type Snapshot } from './services/Desktop';
import './App.css';

const profiles: { id: Profile | null; title: string; percent?: number; text: string }[] = [
  { id: null, title: 'Automático', text: 'A Ayla ajusta os recursos ao seu computador.' },
  { id: 'light', title: 'Leve', percent: 25, text: 'Até 25% da memória. Prioriza economia.' },
  { id: 'moderate', title: 'Moderado', percent: 50, text: 'Até 50% da memória. Equilibra recursos e capacidade.' },
  { id: 'maximum', title: 'Máximo', percent: 75, text: 'Até 75% da memória. Prioriza capacidade.' },
];
const sections = [
  { id: 'desktop', label: 'Integração com o sistema' }, { id: 'uso', label: 'Uso e personalidade' }, { id: 'microfone', label: 'Microfone' }, { id: 'notificacoes', label: 'Notificações' }, { id: 'nuvem', label: 'Nuvem' },
  { id: 'memoria', label: 'Memória' }, { id: 'orgaos', label: 'Órgãos' }, { id: 'avancado', label: 'Opções avançadas' }, { id: 'sessao', label: 'Encerrar ou reiniciar' },
];
type Page = 'models' | 'capabilities' | 'settings';
/** Espaço aberto: a conversa geral ("ayla"), um órgão ("moneta") ou as configurações. */
type SpaceId = 'ayla' | 'settings' | 'moneta';

export default function App() {
  const [space, setSpace] = useState<SpaceId>('ayla'); const [page, setPage] = useState<Page>('settings');
  // Projeto aberto dentro da Ayla; projetos já abertos continuam montados
  // (conversas e respostas em andamento não se perdem ao trocar).
  const [openProject, setOpenProject] = useState<string | null>(null);
  const [visited, setVisited] = useState<string[]>([]);
  const [creating, setCreating] = useState(false); const [projectName, setProjectName] = useState(''); const [projectAbout, setProjectAbout] = useState(''); const [projectError, setProjectError] = useState('');
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [draft, setDraft] = useState<Preferences | null>(null); const [error, setError] = useState(''); const [notice, setNotice] = useState(''); const [saving, setSaving] = useState(false);
  const [key, setKey] = useState(''); const [remember, setRemember] = useState(false);
  const [desktopError, setDesktopError] = useState('');
  const organ = useOrgan(); const [organVisited, setOrganVisited] = useState(false); const [askOrgan, setAskOrgan] = useState(false);
  const organInstalled = Boolean(organ.status?.installed);
  useEffect(() => { if (!organInstalled) { setOrganVisited(false); setSpace(prev => prev === 'moneta' ? 'ayla' : prev); } }, [organInstalled]);
  const onQuick = useCallback(() => { setSpace('ayla'); setDesktopError(''); }, []);
  const view = useDesktopView(onQuick, setDesktopError);
  const panelReady = Boolean((snapshot && draft) || error);
  useLayoutEffect(() => {
    if (panelReady && view.quick && view.pending != null) {
      void native('quick_ready', { generation: view.pending }).catch(e => setDesktopError(errorText(e)));
    }
  }, [panelReady, view.quick, view.pending]);
  function desktopAction(action: string) {
    void native('desktop_action', {action}).catch(e => setDesktopError(errorText(e)));
  }
  const refresh = useCallback(async () => {
    try { const value = await desktopSnapshot(); setSnapshot(value); setDraft(prev => prev || value.preferences); }
    catch (e) { setError(errorText(e)); }
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  // O preparo automático começa no núcleo ao abrir; acompanhar enquanto estiver ativo.
  const preparing = Boolean(snapshot?.setup?.active) || (snapshot !== null && snapshot.preferences.usage_mode !== 'cloud_only' && !snapshot.models.some(m => supports(m, 'text')));
  useEffect(() => {
    if (!preparing) return;
    let timer: ReturnType<typeof setInterval> | undefined;
    function sync() {
      clearInterval(timer);
      if (!document.hidden) { void refresh(); timer = setInterval(() => void refresh(), 1000); }
    }
    sync();
    document.addEventListener('visibilitychange', sync);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', sync); };
  }, [preparing, refresh]);
  function openSpace(id: SpaceId) {
    setSpace(id); setNotice(''); setError('');
    if (id === 'moneta') setOrganVisited(true);
  }
  function showProject(id: string | null) {
    setSpace('ayla'); setOpenProject(id);
    if (id) setVisited(prev => prev.includes(id) ? prev : [...prev, id]);
  }
  function go(next: Page, section?: string) {
    setSpace('settings'); setPage(next); setNotice(''); setError('');
    if (section) setTimeout(() => document.getElementById(section)?.scrollIntoView({ behavior: 'smooth', block: 'start' }), 0);
  }
  function update(values: Partial<Preferences>) { setDraft(prev => prev ? { ...prev, ...values } : prev); setNotice(''); }
  /** Salva na hora uma mudança simples (interruptores e perfil), sem mexer no que está em edição. */
  async function quickSave(values: Partial<Preferences>, message = '') {
    if (!snapshot || saving) return; setSaving(true); setError(''); setNotice('');
    try { await savePreferences({ ...snapshot.preferences, ...values, onboarding_complete: true }); setDraft(prev => prev ? { ...prev, ...values } : prev); await refresh(); if (message) setNotice(message); }
    catch (e) { setError(errorText(e)); } finally { setSaving(false); }
  }
  async function apply(message = 'Preferências salvas.') {
    if (!draft || saving) return; setSaving(true); setNotice(''); setError('');
    try { const value = { ...draft, onboarding_complete: true }; await savePreferences(value); setDraft(value); await refresh(); setNotice(message); }
    catch (e) { setError(errorText(e)); } finally { setSaving(false); }
  }
  async function saveKey() {
    setSaving(true); setError('');
    try { await native('cloud_key', { key, remember, endpoint: draft?.cloud_endpoint || '' }); setKey(''); setNotice(remember ? 'Chave guardada no cofre do sistema.' : 'Chave disponível somente nesta sessão.'); }
    catch (e) { setError(errorText(e)); } finally { setSaving(false); }
  }
  function closeNewProject() { setCreating(false); setProjectName(''); setProjectAbout(''); setProjectError(''); }
  async function createProject(event: FormEvent) {
    event.preventDefault(); setProjectError('');
    try { const project = await native<Project>('create_project', { name: projectName, description: projectAbout }); await refresh(); closeNewProject(); showProject(project.id); }
    catch (e) { setProjectError(errorText(e)); }
  }
  function toggleVoice(voice: boolean) {
    void quickSave({ voice }).then(() => { if (voice) void native('setup_start').catch(err => setError(errorText(err))).finally(() => void refresh()); });
  }

  if (!snapshot || !draft) return <div className="app-shell"><main className="loading">{error ? <p role="alert" className="app-error">{error}</p> : <p role="status">Preparando seu espaço…</p>}</main></div>;

  const prefs = snapshot.preferences;
  const providerSaved = Boolean(draft.cloud_endpoint) && draft.cloud_endpoint === prefs.cloud_endpoint;
  const automatic = prefs.profile === null && prefs.resource_percent === null;
  const current = automatic ? profiles[0] : profiles.find(p => p.id === prefs.profile && prefs.resource_percent === null);
  const profile = draft.profile || snapshot.recommended_profile || 'moderate';
  const transcriber = snapshot.models.some(m => supports(m, 'transcription'));

  return <div className={view.quick ? 'app-shell quick-view' : 'app-shell'} style={view.quick ? quickStyle(view.theme, prefs.desktop) : undefined}>
    {view.quick && <div className="quick-toolbar"><span>Visão rápida</span><button type="button" className="icon-button quick-expand" title="Abrir janela da Ayla" aria-label="Abrir janela da Ayla" onClick={() => desktopAction('open')}><ExpandIcon size={18} /></button></div>}
    {desktopError && <div className="desktop-error" role="alert">{desktopError} <button onClick={()=>setDesktopError('')}>Fechar</button></div>}
    <nav className="rail" aria-label="Espaços">
      <button className="rail-item" aria-label="Ayla" title="Ayla" aria-current={space === 'ayla' ? 'page' : undefined} onClick={() => openSpace('ayla')}><span className="orb" aria-hidden="true" /></button>
      {/* Órgãos: sempre visíveis; apagados enquanto não baixados, e o clique oferece a instalação. */}
      <div className="rail-projects">
        {organ.status && <>
          <span className="rail-divider" aria-hidden="true" />
          <button className={organ.status.installed ? 'rail-item' : 'rail-item organ-missing'} aria-label={organ.status.installed ? organ.status.name : `${organ.status.name} (não instalada)`} title={organ.status.installed ? organ.status.name : `${organ.status.name} · não instalada`} aria-current={space === 'moneta' ? 'page' : undefined}
            onClick={() => organ.status?.installed ? openSpace('moneta') : setAskOrgan(true)}><OrganIcon status={organ.status} /></button>
        </>}
      </div>
      <button className="rail-item" aria-label="Configurações" title="Configurações" aria-current={space === 'settings' ? 'page' : undefined} onClick={() => go('settings')}><SettingsIcon /></button>
    </nav>

    <div className="spaces">
      <div className="page" hidden={space !== 'ayla' || openProject !== null}>
        <Space compact={view.quick} snapshot={snapshot} refresh={refresh} project={null} active={space === 'ayla' && openProject === null} onOpenSettings={section => go('settings', section)}
          projects={snapshot.projects} onOpenProject={showProject} onNewProject={() => setCreating(true)} />
      </div>
      {visited.map(id => {
        const project = snapshot.projects.find(item => item.id === id);
        if (!project) return null;
        const shown = space === 'ayla' && openProject === id;
        return <div key={id} className="page" hidden={!shown}><Space compact={view.quick} snapshot={snapshot} refresh={refresh} project={project} active={shown} onOpenSettings={section => go('settings', section)}
          onBack={() => showProject(null)} onDeleted={() => { setVisited(prev => prev.filter(item => item !== id)); showProject(null); }} /></div>;
      })}

      {organ.status?.installed && organVisited && !view.quick && <div className="page" hidden={space !== 'moneta'}>
        <OrganSpace status={organ.status} active={space === 'moneta'} refresh={organ.refresh} onOpenSettings={() => go('settings', 'orgaos')} />
      </div>}

    {space === 'settings' && <div className="settings-layout">
      <aside className="sidebar settings-sidebar" aria-label="Seções das configurações">
        <div className="brand"><span className="brand-name">Configurações</span></div>
        <nav className="conversation-list" aria-label="Seções">
          {sections.map(item => <button key={item.id} className="conversation-item" onClick={() => go('settings', item.id)}>{item.label}</button>)}
        </nav>
      </aside>

      <main className="settings-main">
        <div className="settings-content">
          {page === 'models' && <><button className="ghost-button back-button" onClick={() => go('settings', 'avancado')}><ChevronLeftIcon size={18} />Configurações</button><Models snapshot={snapshot} refresh={refresh} /></>}
          {page === 'capabilities' && <><button className="ghost-button back-button" onClick={() => go('settings', 'avancado')}><ChevronLeftIcon size={18} />Configurações</button><Capabilities snapshot={snapshot} refresh={refresh} /></>}

          {page === 'settings' && <>
            <header className="settings-header"><h1>Configurações</h1><p>A Ayla cuida das escolhas do dia a dia. Mude só o que quiser.</p></header>
            {error && <p role="alert" className="app-error">{error}</p>}

            <section id="uso" className="settings-group" aria-labelledby="uso-title"><h2 id="uso-title">Uso e personalidade</h2>
              <div className="group-card">
                <fieldset className="group-row stacked">
                  <legend className="row-title">Preferência de uso</legend>
                  <span className="row-hint">Quanto do computador a Ayla pode usar. Mudar aqui não baixa nem carrega modelos.</span>
                  <div className="segmented">{profiles.map(item => <label key={item.title}><input type="radio" name="profile" checked={current === item} disabled={saving} onChange={() => void quickSave({ profile: item.id, resource_percent: null }, `Preferência de uso: ${item.title}.`)} /><span>{item.title}</span></label>)}</div>
                  <span className="row-hint">{current ? current.text : `Personalizado: até ${prefs.resource_percent ?? profiles.find(p => p.id === prefs.profile)?.percent}% da memória.`}{automatic && ` Sugestão para este computador: ${profiles.find(p => p.id === snapshot.recommended_profile)?.title}.`}</span>
                </fieldset>
                <div className="group-row stacked">
                  <label className="row-title" htmlFor="personality">Personalidade</label>
                  <span className="row-hint">Como você quer que a Ayla converse: tom, humor e nível de detalhe. Não muda as regras de privacidade.</span>
                  <textarea id="personality" rows={4} maxLength={4000} value={draft.personality} placeholder="Ex.: seja acolhedora, direta e use humor leve. Explique com exemplos simples." onChange={e => update({ personality: e.target.value })} />
                  <div className="row-actions">
                    <button className="primary-action" disabled={saving || draft.personality === prefs.personality} onClick={() => void apply('Personalidade salva.')}>Salvar</button>
                    <button className="ghost-button" disabled={saving || !prefs.personality.trim()} onClick={() => { setDraft(prev => prev ? { ...prev, personality: '' } : prev); void quickSave({ personality: '' }, 'Personalidade original restaurada.'); }}>Restaurar a original</button>
                  </div>
                </div>
              </div>
            </section>

            <DesktopSettings value={prefs.desktop} saving={saving} save={desktop => void quickSave({desktop})} run={desktopAction} />
            <section id="microfone" className="settings-group" aria-labelledby="mic-title"><h2 id="mic-title">Microfone</h2>
              <div className="group-card">
                <label className="group-row">
                  <span className="row-text"><span className="row-title">Falar com a Ayla</span><span className="row-hint">{prefs.voice === false ? 'A transcrição roda neste computador; a Ayla responde em texto.' : transcriber ? 'Pronto neste computador. A Ayla responde em texto.' : snapshot.setup?.active ? 'Preparando o microfone…' : 'Ativado, mas ainda falta preparar a transcrição.'}</span></span>
                  <input type="checkbox" role="switch" className="switch" checked={prefs.voice !== false} disabled={saving} onChange={e => toggleVoice(e.target.checked)} />
                </label>
                {prefs.voice !== false && !transcriber && !snapshot.setup?.active && <div className="group-row"><span className="row-text"><span className="row-title">Transcrição</span></span><button onClick={() => void native('setup_start').catch(err => setError(errorText(err))).finally(() => void refresh())}>Preparar agora</button></div>}
              </div>
            </section>

            <section id="notificacoes" className="settings-group" aria-labelledby="notify-title"><h2 id="notify-title">Notificações</h2>
              <div className="group-card">
                <label className="group-row">
                  <span className="row-text"><span className="row-title">Resposta pronta</span><span className="row-hint">Quando você estiver em outra janela</span></span>
                  <input type="checkbox" role="switch" className="switch" checked={prefs.notifications.reply} disabled={saving} onChange={e => void quickSave({ notifications: { ...prefs.notifications, reply: e.target.checked } })} />
                </label>
                <label className="group-row">
                  <span className="row-text"><span className="row-title">Preparo de modelos</span><span className="row-hint">Quando um download termina ou dá erro</span></span>
                  <input type="checkbox" role="switch" className="switch" checked={prefs.notifications.setup} disabled={saving} onChange={e => void quickSave({ notifications: { ...prefs.notifications, setup: e.target.checked } })} />
                </label>
              </div>
              <p className="group-note">As notificações nunca mostram o conteúdo das conversas.</p>
            </section>

            <CloudSettings snapshot={snapshot} saving={saving} save={quickSave} />

            <MemoryPanel snapshot={snapshot} refresh={refresh} />

            <OrganSettings status={organ.status} refresh={organ.refresh} />

            <section id="avancado" className="settings-group" aria-labelledby="advanced-title"><h2 id="advanced-title">Opções avançadas</h2>
              <div className="group-card">
                <button className="group-row link-row" onClick={() => go('models')}><span className="row-text"><span className="row-title">Modelos e componentes</span><span className="row-hint">Baixar, importar e remover modelos e motores</span></span><ChevronRightIcon size={18} /></button>
                <button className="group-row link-row" onClick={() => go('capabilities')}><span className="row-text"><span className="row-title">Ferramentas de transcrição e memória</span><span className="row-hint">Transcrever arquivos e buscar memórias por significado</span></span><ChevronRightIcon size={18} /></button>
              </div>
              <details className="group-card advanced-details">
                <summary>Recursos, nuvem e dados</summary>
                <div className="advanced-block"><h3>Seu computador</h3>
                  <p><b>{gib(snapshot.hardware.ram_total_bytes)} GB</b> de memória · <b>{snapshot.hardware.logical_cpus}</b> processadores lógicos</p>
                  <p>{snapshot.hardware.gpus.length ? snapshot.hardware.gpus.map(g => `${g.vendor}${g.vram_total_bytes ? ` · ${gib(g.vram_total_bytes)} GB gráficos detectados` : ' · memória gráfica não identificada'}`).join(' / ') : 'Memória gráfica não identificada. Isso não impede o uso da Ayla.'}</p>
                  <p>Orçamento disponível agora: {formatBytes(snapshot.memory_budget_bytes)}, com reserva para o sistema.</p>
                  <button onClick={() => void refresh()}>Atualizar</button>
                </div>
                <div className="advanced-block"><h3>Limites técnicos</h3>
                  <label>Limite personalizado de memória (%)<input type="number" min={5} max={90} value={draft.resource_percent ?? profiles.find(p => p.id === profile)?.percent ?? 50} onChange={e => update({ resource_percent: Number(e.target.value) })} /></label>
                  <label>Contexto máximo do modelo (tokens)<input type="number" min={512} max={32768} step={512} value={draft.context_tokens} onChange={e => update({ context_tokens: Number(e.target.value) })} /></label>
                  <label>Liberar modelo após inatividade (segundos)<input type="number" min={0} max={3600} value={draft.idle_seconds} onChange={e => update({ idle_seconds: Number(e.target.value) })} /></label>
                  <label>Mensagens recentes no contexto<input type="number" min={2} max={200} value={draft.history_messages} onChange={e => update({ history_messages: Number(e.target.value) })} /></label>
                  <label>Limite de caracteres do histórico<input type="number" min={1000} max={200000} value={draft.history_chars} onChange={e => update({ history_chars: Number(e.target.value) })} /></label>
                  <p>O orçamento limita a estimativa de carregamento; o consumo real varia entre arquiteturas. Respostas interrompidas ficam identificadas e não entram no contexto.</p>
                  <button className="primary-action" disabled={saving} onClick={() => void apply('Limites salvos.')}>Salvar limites</button>
                </div>
                <div className="advanced-block"><h3>Outro provedor compatível</h3>
                  <p>Para um serviço compatível com a API da OpenAI que não está na lista de Nuvem. A nuvem recebe só o pedido e a personalidade, revisados e declarados públicos.</p>
                  <label>Endereço HTTPS<input type="url" value={draft.cloud_endpoint} onChange={e => update({ cloud_endpoint: e.target.value })} placeholder="https://seu-provedor/v1" /></label>
                  <label>Modelo<input value={draft.cloud_model} onChange={e => update({ cloud_model: e.target.value })} /></label>
                  <button className="primary-action" disabled={saving || !draft.cloud_endpoint} onClick={() => { const value = { ...draft, cloud_provider: 'custom', onboarding_complete: true }; setDraft(value); void savePreferences(value).then(() => refresh()).then(() => setNotice('Provedor próprio salvo e em uso.')).catch(e => setError(errorText(e))); }}>Usar este provedor</button>
                  <label>Chave do provedor<input type="password" autoComplete="off" value={key} onChange={e => setKey(e.target.value)} /></label>
                  <label className="check"><input type="checkbox" checked={remember} onChange={e => setRemember(e.target.checked)} />Guardar no cofre do sistema</label>
                  <div className="row-actions">
                    <button disabled={saving || !key || !providerSaved} onClick={() => void saveKey()}>{remember ? 'Guardar chave no cofre' : 'Usar chave nesta sessão'}</button>
                    <button disabled={saving || !providerSaved} onClick={() => { void native('cloud_key', { key: '', remember: false, endpoint: draft.cloud_endpoint }).then(() => setNotice('Chave removida.')).catch(e => setError(errorText(e))); }}>Esquecer chave deste provedor</button>
                  </div>
                </div>
                <div className="advanced-block"><h3>Dados</h3>
                  <p>As conversas ficam isoladas no banco local. O histórico antigo permanece guardado, sem resumos automáticos.</p>
                  <div className="row-actions">
                    <button onClick={() => { void native('backup_data').then(() => setNotice('Backup criado na pasta de dados da Ayla.')).catch(e => setError(errorText(e))); }}>Criar backup</button>
                    <button onClick={() => { void native<number | null>('import_legacy').then(count => { if (count !== null) setNotice(`Histórico importado: ${count} mensagens em uma conversa separada. Fonte original preservada.`); }).catch(e => setError(errorText(e))); }}>Importar histórico anterior</button>
                  </div>
                </div>
                <div className="advanced-block"><h3>Medições locais</h3>
                  <p>Tempo até o primeiro trecho, duração e quantidade de trechos das últimas respostas.</p>
                  {!snapshot.metrics.length && <p>Ainda sem medições.</p>}
                  {snapshot.metrics.slice(0, 5).map((m, i) => <p key={i}>Primeiro trecho: {m.first_token_ms ?? '—'} ms · duração: {m.duration_ms} ms · {m.chunks} trechos{m.cancelled ? ' · interrompida' : ''}</p>)}
                </div>
              </details>
            </section>

            <section id="sessao" className="settings-group" aria-labelledby="session-title"><h2 id="session-title">Encerrar ou reiniciar</h2>
              <div className="group-card">
                <div className="group-row"><span className="row-text"><span className="row-title">Reiniciar a Ayla</span><span className="row-hint">Fecha e abre de novo, com conversas e configurações guardadas.{organ.status?.container ? ' A Moneta reinicia junto.' : ''}</span></span>
                  <button onClick={() => desktopAction('restart')}>Reiniciar</button></div>
                <div className="group-row"><span className="row-text"><span className="row-title">Encerrar a Ayla</span><span className="row-hint">Fecha a Ayla, inclusive em segundo plano.{organ.status?.container ? ' A Moneta aberta por ela também é encerrada.' : ''}</span></span>
                  <button onClick={() => desktopAction('quit')}>Encerrar</button></div>
              </div>
            </section>
          </>}
          {notice && <p role="status" className="notice">{notice}</p>}
        </div>
      </main>
    </div>}
    </div>

    {askOrgan && organ.status && <OrganInstallDialog status={organ.status} refresh={organ.refresh} onClose={() => setAskOrgan(false)} onInstalled={() => { setAskOrgan(false); openSpace('moneta'); }} />}

    {creating && <div className="dialog-backdrop" onClick={closeNewProject}>
      <form className="dialog" role="dialog" aria-modal="true" aria-labelledby="new-project-title" onSubmit={e => void createProject(e)} onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape') closeNewProject(); }}>
        <h2 id="new-project-title">Novo projeto</h2>
        <p>Um espaço com conversas e memória próprias, focado em algo específico. O que a Ayla aprende nele fica nele.</p>
        <label>Nome<input autoFocus value={projectName} maxLength={80} onChange={e => setProjectName(e.target.value)} placeholder="Ex.: Mudança para Recife" /></label>
        <label>Sobre o projeto (opcional)<textarea rows={3} maxLength={2000} value={projectAbout} onChange={e => setProjectAbout(e.target.value)} placeholder="Do que se trata, objetivos, prazos…" /></label>
        {projectError && <p role="alert" className="app-error">{projectError}</p>}
        <div className="row-actions"><button className="primary-action" disabled={!projectName.trim()}>Criar projeto</button><button type="button" className="ghost-button" onClick={closeNewProject}>Cancelar</button></div>
      </form>
    </div>}
  </div>;
}

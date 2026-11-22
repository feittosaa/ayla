import { useCallback, useEffect, useState } from 'react';
import Confirm from './Confirm';
import { native, errorText, modeLabels, taskId, type CloudProvider, type Preferences, type Snapshot, type UsageMode } from '../services/Desktop';

type Props = {
  snapshot: Snapshot; saving: boolean;
  /** Salva na hora (mesma função dos outros interruptores das Configurações). */
  save: (values: Partial<Preferences>, message?: string) => Promise<void>;
};
const USAGE = 'https://chatgpt.com/settings/usage';

/** Nuvem: 100% local ou também nuvem, contas conectadas e plataforma/modelo padrão. */
export default function CloudSettings({ snapshot, saving, save }: Props) {
  const prefs = snapshot.preferences;
  const enabled = prefs.usage_mode !== 'local_only';
  const [providers, setProviders] = useState<CloudProvider[] | null>(null);
  const [keys, setKeys] = useState<Record<string, string>>({}); const [remember, setRemember] = useState(true);
  const [busy, setBusy] = useState<{ id: string; request?: string } | null>(null); const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const load = useCallback(() => native<CloudProvider[]>('cloud_providers').then(setProviders).catch(e => setError(errorText(e))), []);
  useEffect(() => { if (enabled) void load(); }, [enabled, load]);
  const current = providers?.find(p => p.id === prefs.cloud_provider);
  // Imagens: plataforma conectada que cria imagens e o modelo escolhido.
  const imageProviders = providers?.filter(p => p.images && p.connected) ?? [];
  const imageCurrent = providers?.find(p => p.id === prefs.image_provider && p.connected);
  const [imageModels, setImageModels] = useState<{ id: string; name: string }[] | null>(null);
  const [imageModelText, setImageModelText] = useState(prefs.image_model);
  useEffect(() => { setImageModelText(prefs.image_model); }, [prefs.image_model]);
  useEffect(() => {
    if (!imageCurrent) { setImageModels(null); return; }
    let alive = true;
    setImageModels(null);
    native<{ id: string; name: string }[]>('cloud_models', { provider: imageCurrent.id, images: true })
      .then(list => { if (alive) setImageModels(list); })
      .catch(e => { if (alive) { setImageModels([]); setError(errorText(e)); } });
    return () => { alive = false; };
  }, [imageCurrent?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (imageModels?.length && !prefs.image_model) void save({ image_model: imageModels[0].id }); }, [imageModels]); // eslint-disable-line react-hooks/exhaustive-deps

  const link = (url: string) => { void native('open_link', { url }).catch(e => setError(errorText(e))); };
  async function act(id: string, work: () => Promise<string | void>) {
    setBusy({ id }); setError(''); setNotice('');
    try { const message = await work(); if (message) setNotice(message); await load(); } catch (e) { setError(errorText(e)); } finally { setBusy(null); }
  }
  async function login(provider: CloudProvider) {
    const request = taskId(); setBusy({ id: provider.id, request }); setError(''); setNotice('');
    try {
      const result = await native<{ account: string | null; warning: string | null }>('cloud_login', { provider: provider.id, requestId: request });
      setNotice(result.warning || (provider.login === 'chat_gpt' ? 'Você está usando o seu plano do ChatGPT: pedidos elegíveis usam o limite do plano (no Plus, o limite de 5 horas é compartilhado com outros apps).' : `${provider.name} conectado.`));
      await load();
      if (!prefs.cloud_provider) await choose(provider);
    } catch (e) { setError(errorText(e)); } finally { setBusy(null); }
  }
  async function choose(provider: CloudProvider) {
    await save({ cloud_provider: provider.id, cloud_model: provider.id === prefs.cloud_provider ? prefs.cloud_model : (provider.default_model ?? '') }, `${provider.name} será usada nas respostas pela nuvem.`);
  }

  return <section id="nuvem" className="settings-group" aria-labelledby="cloud-title"><h2 id="cloud-title">Nuvem</h2>
    <div className="group-card">
      <label className="group-row">
        <span className="row-text"><span className="row-title">Usar IA na nuvem</span><span className="row-hint">{enabled ? 'Conteúdo sensível ou não verificado continua neste computador. No modo Automático, pedidos classificados como públicos seguem direto para a conta conectada.' : 'Desligado: a Ayla funciona 100% neste computador.'}</span></span>
        <input type="checkbox" role="switch" className="switch" checked={enabled} disabled={saving} onChange={e => void save({ usage_mode: e.target.checked ? 'explicit_request' : 'local_only', cloud_asked: true }, e.target.checked ? 'Nuvem ligada. Conecte uma conta abaixo.' : 'A Ayla agora funciona 100% neste computador.')} />
      </label>
      {enabled && <label className="group-row">
        <span className="row-text"><span className="row-title">Quando usar</span></span>
        <select value={prefs.usage_mode} disabled={saving} onChange={e => void save({ usage_mode: e.target.value as UsageMode })}>{Object.entries(modeLabels).filter(([mode]) => mode !== 'local_only').map(([mode, label]) => <option key={mode} value={mode}>{label}</option>)}</select>
      </label>}
    </div>
    {error && <p role="alert" className="app-error">{error}</p>}
    {notice && <p role="status" className="group-note">{notice}{notice.includes('plano do ChatGPT') && <> <button className="link-button" onClick={() => link(USAGE)}>Gerenciar uso</button></>}</p>}

    {enabled && <>
      <h2 className="subgroup-title">Contas</h2>
      <div className="group-card">
        {!providers && <div className="group-row"><span className="row-hint">Carregando…</span></div>}
        {providers?.map(provider => {
          const working = busy?.id === provider.id;
          const selected = prefs.cloud_provider === provider.id;
          return <div key={provider.id} className="group-row stacked cloud-row">
            <div className="cloud-head">
              <span className="row-text"><span className="row-title">{provider.name}{selected && <span className="badge">Em uso</span>}</span>
                <span className="row-hint">{provider.connected ? (provider.account ? `Conectado como ${provider.account}` : 'Conectado') : provider.summary}</span></span>
              {provider.connected ? <span className="row-actions">
                {!selected && <button disabled={saving || working} onClick={() => void choose(provider)}>Usar</button>}
                <Confirm className="ghost-button" disabled={working} question={provider.login === 'key' ? `Remover a chave de ${provider.name} deste computador?` : `Sair da conta ${provider.name}? O acesso é revogado.`} confirm={provider.login === 'key' ? 'Remover' : 'Sair'} onConfirm={() => void act(provider.id, async () => { await native('cloud_disconnect', { provider: provider.id }); if (selected) await save({ cloud_provider: '', cloud_model: '' }); return `${provider.name} desconectado.`; })}>{provider.login === 'key' ? 'Remover chave' : 'Sair'}</Confirm>
              </span> : provider.login !== 'key' && (working && busy?.request
                ? <span className="row-actions"><span className="row-hint" role="status">Termine o login no navegador…</span><button className="ghost-button" onClick={() => void native('cancel_job', { requestId: busy.request })}>Cancelar</button></span>
                : <button className={provider.login === 'chat_gpt' ? 'primary-action' : ''} disabled={!!busy} onClick={() => void login(provider)}>{provider.login === 'chat_gpt' ? 'Continuar com o ChatGPT' : 'Entrar com OpenRouter'}</button>)}
            </div>
            {!provider.connected && provider.login === 'key' && <form className="key-form" onSubmit={e => { e.preventDefault(); const key = keys[provider.id] || ''; void act(provider.id, async () => { await native('cloud_connect', { provider: provider.id, key, remember }); setKeys(k => ({ ...k, [provider.id]: '' })); if (!prefs.cloud_provider) await choose({ ...provider, connected: true }); return `${provider.name} conectado.`; }); }}>
              <input type="password" autoComplete="off" aria-label={`Chave de API da ${provider.name}`} placeholder="Cole a chave de API" value={keys[provider.id] || ''} onChange={e => setKeys(k => ({ ...k, [provider.id]: e.target.value }))} />
              <button className="primary-action" disabled={working || !(keys[provider.id] || '').trim()}>Salvar</button>
              {provider.keys_url && <button type="button" className="ghost-button" onClick={() => link(provider.keys_url!)}>Obter chave</button>}
            </form>}
          </div>;
        })}
        {providers && <label className="group-row check"><input type="checkbox" checked={remember} onChange={e => setRemember(e.target.checked)} />Guardar chaves no cofre do sistema (senão valem só até fechar a Ayla)</label>}
      </div>

      {current?.connected && current.login === 'chat_gpt' && <div className="group-card">
        <div className="group-row"><span className="row-hint">Você está usando o seu plano do ChatGPT. <button className="link-button" onClick={() => link(USAGE)}>Gerenciar uso</button></span></div>
      </div>}

      {providers && <>
        <h2 className="subgroup-title">Imagens</h2>
        <div className="group-card">
          <div className="group-row"><span className="row-hint">A Ayla só cria imagens quando você pede (botão de imagem ou "gere uma imagem de…"), pela nuvem e depois da sua revisão. A imagem fica guardada na conversa. O plano do ChatGPT não cria imagens em outros apps.</span></div>
          {!imageProviders.length ? <div className="group-row"><span className="row-hint">Conecte OpenAI, Google Gemini, xAI ou OpenRouter acima para criar imagens.</span></div> : <>
            <label className="group-row">
              <span className="row-text"><span className="row-title">Plataforma</span></span>
              <select value={imageCurrent ? prefs.image_provider : ''} disabled={saving} onChange={e => void save({ image_provider: e.target.value, image_model: '' })}><option value="">Nenhuma</option>{imageProviders.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}</select>
            </label>
            {imageCurrent && <label className="group-row stacked">
              <span className="row-title">Modelo de imagem</span>
              {imageModels && imageModels.length > 0
                ? <select value={prefs.image_model} disabled={saving} onChange={e => void save({ image_model: e.target.value })}>{!imageModels.some(m => m.id === prefs.image_model) && prefs.image_model && <option value={prefs.image_model}>{prefs.image_model}</option>}{imageModels.map(m => <option key={m.id} value={m.id}>{m.name}</option>)}</select>
                : <input value={imageModelText} placeholder={imageModels ? 'Nome do modelo de imagem' : 'Carregando modelos…'} disabled={!imageModels} onChange={e => setImageModelText(e.target.value)} onBlur={() => { if (imageModelText.trim() !== prefs.image_model) void save({ image_model: imageModelText.trim() }); }} />}
            </label>}
          </>}
        </div>
      </>}
    </>}
  </section>;
}

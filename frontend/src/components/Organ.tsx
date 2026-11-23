import { useCallback, useEffect, useRef, useState } from 'react';
import { errorText, native } from '../services/Desktop';
import { monetaPage, postAppearance, type OrganStatus } from '../services/Organ';

export function OrganIcon({ status, size = 26 }: { status: OrganStatus; size?: number }) {
  // Logo monocromático (SVG) na cor do texto, como as marcas da Ayla, Athena e Iris.
  return status.icon ? <span className="brand-mark" aria-hidden="true" style={{ width: size, height: size, WebkitMaskImage: `url("${status.icon}")`, maskImage: `url("${status.icon}")` }} /> : <span className="avatar" style={{ width: size, height: size, background: '#d8b45a' }}>{status.name.slice(0, 1)}</span>;
}

/** Espaço do órgão: inicia o contêiner da Moneta e a mostra na mesma tela da Ayla. */
export function OrganSpace({ status, active, refresh, onOpenSettings }: { status: OrganStatus; active: boolean; refresh: () => Promise<void>; onOpenSettings: () => void }) {
  const [ready, setReady] = useState(false); const [error, setError] = useState(''); const [starting, setStarting] = useState(false);
  const start = useCallback(async () => {
    setStarting(true); setError('');
    try { await native('organ_start'); setReady(true); } catch (e) { setError(errorText(e)); } finally { setStarting(false); void refresh(); }
  }, [refresh]);
  useEffect(() => { if (active && !ready && !starting && !error) void start(); }, [active, ready, starting, error, start]);
  // A página carrega invisível por baixo da espera e só aparece pronta, com a mesma entrada suave
  // dos outros espaços (sem o quadro em branco da primeira abertura).
  const [loaded, setLoaded] = useState(false);
  const frame = useRef<HTMLIFrameElement>(null);
  useEffect(() => {
    const update = () => postAppearance(frame.current);
    window.addEventListener('ayla-tema', update);
    return () => window.removeEventListener('ayla-tema', update);
  }, []);
  const waiting = <div className="organ-space organ-waiting">
    <OrganIcon status={status} size={48} />
    <h1>{status.name}</h1>
    {error ? <>
      <p role="alert" className="app-error">{error}</p>
      <div className="row-actions"><button className="primary-action" onClick={() => void start()}>Tentar de novo</button><button className="ghost-button" onClick={onOpenSettings}>Ver em Configurações</button></div>
    </> : <p role="status">{ready ? `Abrindo a ${status.name}…` : status.task ?? 'Iniciando a Moneta…'}</p>}
  </div>;
  // Uma vez carregada, a página continua montada: trocar de espaço não perde o que estava aberto.
  if (!ready) return waiting;
  return <div className="organ-space organ-stack">
    <iframe ref={frame} className={loaded ? 'organ-frame shown' : 'organ-frame'} title={status.name} src={monetaPage} onLoad={() => { postAppearance(frame.current); requestAnimationFrame(() => requestAnimationFrame(() => setLoaded(true))); }} />
    {!loaded && waiting}
  </div>;
}

/** Configurações › Órgãos: baixar ou remover a Moneta junto da Ayla. */
export function OrganSettings({ status, refresh }: { status: OrganStatus | null; refresh: () => Promise<void> }) {
  const [busy, setBusy] = useState(false); const [notice, setNotice] = useState(''); const [error, setError] = useState(''); const [confirm, setConfirm] = useState(false);
  if (!status) return null;
  async function act(command: string, done: string) {
    setBusy(true); setError(''); setNotice(''); setConfirm(false);
    const poll = setInterval(() => void refresh(), 1500);
    try { await native(command); setNotice(done); } catch (e) { setError(errorText(e)); } finally { clearInterval(poll); setBusy(false); void refresh(); }
  }
  const hint = status.task ? `${status.task}…`
    : status.standalone ? 'Usando a Moneta instalada neste computador, com os seus dados.'
    : status.installed ? `Versão ${status.version}${status.container ? ' · rodando no contêiner da Ayla' : ''}.`
    : status.description;
  return <section id="orgaos" className="settings-group" aria-labelledby="organs-title"><h2 id="organs-title">Órgãos</h2>
    <div className="group-card">
      <div className="group-row">
        <OrganIcon status={status} size={32} />
        <span className="row-text"><span className="row-title">{status.name}</span><span className="row-hint">{hint}</span></span>
        {status.standalone ? null : status.installed
          ? confirm
            ? <span className="row-actions"><button disabled={busy} onClick={() => void act('organ_remove', 'Moneta removida da Ayla. Seus dados financeiros continuam guardados.')}>Remover</button><button className="ghost-button" onClick={() => setConfirm(false)}>Cancelar</button></span>
            : <button className="ghost-button" disabled={busy || Boolean(status.task)} onClick={() => setConfirm(true)}>Remover</button>
          : <button className="primary-action" disabled={busy || Boolean(status.task) || !status.engine} onClick={() => void act('organ_install', 'Moneta pronta. Ela aparece na lateral da Ayla.')}>Baixar</button>}
      </div>
      {status.installed && <div className="group-row"><span className="row-text"><span className="row-title">Versão separada</span><span className="row-hint">Abre esta Moneta numa janela própria, com os mesmos dados.</span></span>
        <button disabled={busy} onClick={() => { void native('organ_open_separate').catch(e => setError(errorText(e))); }}>Abrir</button></div>}
    </div>
    <p className="group-note">{status.engine
      ? status.standalone
        ? 'A Ayla e a Moneta trabalham juntas: a Moneta aparece na lateral, segue o visual da Ayla e responde às perguntas sobre finanças com os mesmos dados.'
        : 'Opcional. Sem a Moneta instalada neste computador, a Ayla baixa a Moneta do repositório oficial e a roda num contêiner Docker separado; remover daqui nunca apaga os dados.'
      : 'A Moneta roda num contêiner separado e precisa do Docker com Compose. Instale o Docker para poder baixá-la.'}</p>
    {!status.contract_ok && <p role="alert" className="app-error">A Moneta aberta neste computador usa outra versão do contrato com a Ayla. Atualize as duas.</p>}
    {(error || status.error) && <p role="alert" className="app-error">{error || status.error}</p>}
    {notice && <p role="status" className="notice">{notice}</p>}
  </section>;
}

/** Modal ao clicar num órgão ainda não baixado: explica e oferece a instalação. */
export function OrganInstallDialog({ status, refresh, onClose, onInstalled }: { status: OrganStatus; refresh: () => Promise<void>; onClose: () => void; onInstalled: () => void }) {
  const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  async function install() {
    setBusy(true); setError('');
    const poll = setInterval(() => void refresh(), 1500);
    try { await native('organ_install'); await refresh(); onInstalled(); }
    catch (e) { setError(errorText(e)); } finally { clearInterval(poll); setBusy(false); void refresh(); }
  }
  const close = () => { if (!busy) onClose(); };
  return <div className="dialog-backdrop" onClick={close}>
    <div className="dialog" role="dialog" aria-modal="true" aria-labelledby="organ-install-title" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape') close(); }}>
      <div className="organ-dialog-head"><OrganIcon status={status} size={40} /><h2 id="organ-install-title">Instalar a {status.name}?</h2></div>
      <p>{status.description}</p>
      <p>A Ayla baixa a {status.name} do repositório oficial (versão {status.version}) e a roda num contêiner Docker separado, aberta aqui mesmo na lateral. Os dados ficam guardados com a Ayla, e dá para remover depois em Configurações › Órgãos.</p>
      {!status.engine && <p role="alert" className="app-error">A {status.name} precisa do Docker com Compose. Instale o Docker e tente de novo.</p>}
      {busy && <p role="status">{status.task ? `${status.task}…` : 'Preparando…'}</p>}
      {error && <p role="alert" className="app-error">{error}</p>}
      <div className="row-actions">
        <button className="primary-action" autoFocus disabled={busy || !status.engine} onClick={() => void install()}>{busy ? 'Instalando…' : 'Instalar'}</button>
        <button className="ghost-button" disabled={busy} onClick={onClose}>Agora não</button>
      </div>
    </div>
  </div>;
}

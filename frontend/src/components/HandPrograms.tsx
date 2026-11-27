import { invoke, Channel } from '@tauri-apps/api/core';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { errorText, native } from '../services/Desktop';

type Avaliacao = { resumo: string; desinstalar: string; junto: string[]; libera: string; essencial: boolean; modelo: boolean };
type Uso = { ultimo: number | null; desde: number };
type App = { id: string; name: string; comment: string; icone: string | null; origem: 'sistema' | 'flatpak' | 'manual'; pacote: string; versao: string; avaliacao: Avaliacao | null; uso: Uso | null };
type Previa = { origem: App['origem']; pacote: string; sairiam: string[]; libera: string; essencial: boolean; explicacao: string; comando: string | null };

const origens: Record<App['origem'], string> = { sistema: 'Pacote do sistema', flatpak: 'Flatpak', manual: 'Sem pacote' };
const filtros: [string, string][] = [['', 'Todos'], ['sistema', 'Sistema'], ['flatpak', 'Flatpak'], ['manual', 'Outros']];
const plain = (t: string) => t.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
const DAY = 86400;
const SIX_MONTHS = 182 * DAY;
const date = (s: number) => new Date(s * 1000).toLocaleDateString('pt-BR');
/** Última atividade conhecida (uso ou chegada). */
const lastSeen = (u: Uso) => u.ultimo ?? u.desde;
function ago(s: number) {
  const days = Math.floor((Date.now() / 1000 - s) / DAY);
  if (days < 1) return 'hoje';
  if (days < 2) return 'ontem';
  if (days < 45) return `há ${days} dias`;
  const months = Math.round(days / 30);
  return months < 12 ? `há ${months} meses` : `há ${Math.floor(days / 365)} ${Math.floor(days / 365) === 1 ? 'ano' : 'anos'}`;
}
const unused = (u: Uso | null) => !!u && Date.now() / 1000 - lastSeen(u) > SIX_MONTHS;
function usageText(u: Uso | null) {
  if (!u) return 'Último uso: não deu para saber.';
  if (u.ultimo === null) return `Sem uso registrado desde ${date(u.desde)} (instalação ou última atualização).`;
  return `Último uso: ${ago(u.ultimo)} (${date(u.ultimo)}).`;
}

/** Ícone do programa (svg/png do tema de ícones). */
function Icon({ app }: { app: App }) {
  const [url, setUrl] = useState('');
  useEffect(() => {
    if (!app.icone) return;
    let alive = true;
    let made = '';
    const type = app.icone.endsWith('.svg') ? 'image/svg+xml' : 'image/png';
    invoke<ArrayBuffer>('hand_icone', { caminho: app.icone }).then(bytes => { if (!alive) return; made = URL.createObjectURL(new Blob([bytes], { type })); setUrl(made); }).catch(() => {});
    return () => { alive = false; if (made) URL.revokeObjectURL(made); };
  }, [app.icone]);
  return <span className="hand-app-icon" aria-hidden="true">{url ? <img src={url} alt="" draggable={false} /> : <span className="hand-app-letter">{app.name.slice(0, 1).toUpperCase()}</span>}</span>;
}

/** Programas: grade com ícones. Passar o mouse (ou focar) mostra o que o programa faz no sistema
 * e o que aconteceria ao desinstalar, avaliado pela Ayla e guardado por versão. */
export default function HandPrograms({ active, fail }: { active: boolean; fail: (e: unknown) => void }) {
  const [list, setList] = useState<App[] | null>(null);
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('');
  const [usage, setUsage] = useState<'' | 'parado'>('');
  const [progress, setProgress] = useState<{ feitos: number; total: number } | null>(null);
  const [notice, setNotice] = useState('');
  const [hover, setHover] = useState<{ app: App; x: number; y: number; left: boolean } | null>(null);
  const [removing, setRemoving] = useState<App | null>(null);
  const grid = useRef<HTMLDivElement>(null);
  const hideTimer = useRef<number | undefined>(undefined);

  const load = useCallback(() => native<{ programas: App[] }>('hand_programas_grade').then(r => setList(r.programas)).catch(fail), [fail]);
  // Ao entrar: lista na hora e avalia os novos/atualizados em segundo plano.
  useEffect(() => {
    if (!active) return;
    let alive = true;
    void load().then(() => {
      const events = new Channel<{ feitos: number; total: number }>();
      events.onmessage = p => { if (!alive) return; setProgress(p); if (p.feitos > 0) void load(); };
      void native<{ avaliados: number }>('hand_programas_avaliar', { events }).then(() => { if (alive) { setProgress(null); void load(); } }).catch(e => { if (alive) { setProgress(null); fail(e); } });
    });
    return () => { alive = false; };
  }, [active, load, fail]);

  const shown = useMemo(() => {
    const q = plain(query.trim());
    const out = (list ?? []).filter(p => (!filter || p.origem === filter) && (!usage || unused(p.uso))
      && (!q || plain(`${p.name} ${p.comment} ${p.pacote} ${p.avaliacao?.resumo ?? ''}`).includes(q)));
    // Nos filtros de uso, os mais esquecidos primeiro.
    return usage ? [...out].sort((a, b) => (a.uso ? lastSeen(a.uso) : 0) - (b.uso ? lastSeen(b.uso) : 0)) : out;
  }, [list, query, filter, usage]);

  function show(app: App, el: HTMLElement) {
    window.clearTimeout(hideTimer.current);
    const box = grid.current?.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    if (!box) return;
    const left = r.left - box.left + r.width + 300 > box.width;
    setHover({ app, x: left ? r.left - box.left : r.right - box.left, y: r.top - box.top + (grid.current?.scrollTop ?? 0), left });
  }
  const hide = () => { window.clearTimeout(hideTimer.current); hideTimer.current = window.setTimeout(() => setHover(null), 160); };
  const keep = () => window.clearTimeout(hideTimer.current);
  const openApp = (app: App) => void native('hand_abrir_programa', { id: app.id }).then(() => setNotice(`Abrindo ${app.name}…`)).catch(fail);

  return <div className="hand-panel hand-programs">
    <div className="hand-search">
      <input className="organ-search" type="search" autoFocus value={query} placeholder="Buscar programa (nome, pacote ou o que faz)" aria-label="Buscar programa" onChange={e => setQuery(e.target.value)} />
      <div className="hand-options" role="radiogroup" aria-label="Origem">
        {filtros.map(([v, l]) => <label key={v} className={filter === v ? 'hand-chip on' : 'hand-chip'}><input type="radio" name="hand-origem" checked={filter === v} onChange={() => setFilter(v)} />{l}</label>)}
      </div>
    </div>
    <div className="hand-options" role="radiogroup" aria-label="Uso">
      <span className="hand-options-label">Uso</span>
      {([['', 'Qualquer'], ['parado', `Sem uso há 6 meses ou mais (${(list ?? []).filter(p => unused(p.uso)).length})`]] as const).map(([v, l]) =>
        <label key={v} className={usage === v ? 'hand-chip on' : 'hand-chip'}><input type="radio" name="hand-uso" checked={usage === v} onChange={() => setUsage(v)} />{l}</label>)}
    </div>
    {usage && <p className="panel-muted">Pelo último acesso ao programa, que o sistema anota no máximo uma vez por dia (sem uso registrado, conta a instalação ou a última atualização). Os mais esquecidos vêm primeiro.</p>}
    {progress && progress.total > 0 && <p role="status" className="hand-progress">A Ayla está avaliando {progress.total === 1 ? 'um programa novo' : `${progress.total} programas novos ou atualizados`}: {progress.feitos} de {progress.total}.</p>}
    {notice && <p role="status" className="hand-done">{notice}</p>}
    {!list ? <p role="status" className="panel-muted">Lendo os programas…</p>
      : <div className="hand-app-grid-wrap" ref={grid} onScroll={() => setHover(null)}>
        <ul className="hand-app-grid" aria-label="Programas">
          {shown.map(app => <li key={app.id}>
            <button className="hand-app" onMouseEnter={e => show(app, e.currentTarget)} onMouseLeave={hide} onFocus={e => show(app, e.currentTarget)} onBlur={hide} onClick={() => openApp(app)}
              aria-describedby={hover?.app.id === app.id ? 'hand-app-info' : undefined} title="">
              <Icon app={app} /><span className="hand-app-name">{app.name}</span>
              {usage && unused(app.uso) && <span className="hand-app-tag">parado {ago(lastSeen(app.uso!))}</span>}
            </button>
          </li>)}
        </ul>
        {hover && <Info key={hover.app.id} app={hover.app} pos={hover} onEnter={keep} onLeave={hide} onOpen={() => openApp(hover.app)} onRemove={() => { setRemoving(hover.app); setHover(null); }} />}
      </div>}
    {list && !shown.length && <p className="panel-muted">Nenhum programa com esse nome.</p>}
    {removing && <Uninstall app={removing} close={() => setRemoving(null)} done={name => { setRemoving(null); setNotice(`${name} foi desinstalado.`); void load(); }} />}
  </div>;
}

function Info({ app, pos, onEnter, onLeave, onOpen, onRemove }: { app: App; pos: { x: number; y: number; left: boolean }; onEnter: () => void; onLeave: () => void; onOpen: () => void; onRemove: () => void }) {
  const a = app.avaliacao;
  const canRemove = app.origem !== 'manual' && !a?.essencial;
  return <div id="hand-app-info" role="tooltip" className={pos.left ? 'hand-app-info left' : 'hand-app-info'} style={{ top: pos.y, left: pos.x }} onMouseEnter={onEnter} onMouseLeave={onLeave}>
    <header><b>{app.name}</b><small>{origens[app.origem]}{app.pacote ? ` · ${app.pacote}` : ''}{app.versao ? ` ${app.versao}` : ''}</small></header>
    <p>{a?.resumo || app.comment || 'Ainda sem avaliação.'}</p>
    <p className="hand-app-usage">{usageText(app.uso)}</p>
    <p className="hand-app-remove"><span>Se desinstalar:</span> {a ? a.desinstalar : app.origem === 'manual' ? 'Não veio de um pacote: não dá para desinstalar por aqui.' : 'A Ayla ainda está avaliando.'}</p>
    {a && a.modelo && !a.essencial && a.junto.length > 0 && <p className={a.junto.length >= 20 ? 'hand-app-cascade big' : 'hand-app-cascade'}>
      {a.junto.length >= 20 ? 'Leva muita coisa junto: ' : 'Sai junto: '}{a.junto.length} {a.junto.length === 1 ? 'pacote' : 'pacotes'} ({a.junto.slice(0, 4).join(', ')}{a.junto.length > 4 ? '…' : ''}){a.libera ? `, libera ${a.libera}` : ''}.</p>}
    {a && !a.modelo && app.origem !== 'manual' && <small className="panel-muted">Pela descrição do pacote (sem modelo local).</small>}
    <div className="row-actions">
      <button className="ghost-button" onClick={onOpen}>Abrir</button>
      {canRemove && <button className="ghost-button danger-text" onClick={onRemove}>Desinstalar…</button>}
      {a?.essencial && <small className="panel-muted">Parte do sistema: não desinstala.</small>}
    </div>
  </div>;
}

/** Desinstalar em dois passos: primeiro o que sairia (simulação nova), depois a confirmação. */
function Uninstall({ app, close, done }: { app: App; close: () => void; done: (name: string) => void }) {
  const [previa, setPrevia] = useState<Previa | null>(null);
  const [step, setStep] = useState<'ver' | 'confirmar' | 'rodando'>('ver');
  const [error, setError] = useState('');
  useEffect(() => { void native<Previa>('hand_programa_previa', { id: app.id }).then(setPrevia).catch(e => setError(errorText(e))); }, [app.id]);
  async function run() {
    setStep('rodando'); setError('');
    try { await native('hand_programa_desinstalar', { id: app.id }); done(app.name); }
    catch (e) { setError(errorText(e)); setStep('ver'); }
  }
  const outros = previa?.sairiam.filter(p => p !== previa.pacote) ?? [];
  return <div className="dialog-backdrop" onClick={() => step !== 'rodando' && close()}>
    <div className="dialog hand-dialog" role="dialog" aria-modal="true" aria-labelledby="hand-uninstall-title" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape' && step !== 'rodando') close(); }}>
      <h2 id="hand-uninstall-title">Desinstalar {app.name}?</h2>
      {!previa && !error && <p role="status" className="panel-muted">Vendo o que sairia junto (nada muda ainda)…</p>}
      {previa && <>
        <p>{app.avaliacao?.desinstalar && app.avaliacao.modelo ? app.avaliacao.desinstalar : previa.explicacao}</p>
        {outros.length >= 20 && <p className="app-error">Atenção: isso leva {outros.length} pacotes junto. Confira a lista antes.</p>}
        {outros.length > 0 && <><p className="hand-options-label">Sairiam junto ({outros.length}):</p><ul className="hand-pkg-list">{outros.map(p => <li key={p}><code>{p}</code></li>)}</ul></>}
        {previa.libera && <p className="panel-muted">Libera {previa.libera}.</p>}
        {previa.comando && <p className="panel-muted">Vai rodar <code>{previa.comando}</code>{previa.origem === 'sistema' ? '; o sistema pede a sua senha.' : '.'}</p>}
        {previa.origem === 'flatpak' && <p className="panel-muted">Os dados do app continuam em ~/.var/app/{previa.pacote}.</p>}
      </>}
      {error && <p role="alert" className="app-error">{error}</p>}
      <div className="row-actions dialog-actions">
        {step === 'confirmar'
          ? <><span className="confirm-question">Tem certeza? Isso remove {outros.length ? `${outros.length + 1} pacotes` : 'o programa'} do computador.</span><button className="danger-action" onClick={() => void run()}>Sim, desinstalar</button><button className="ghost-button" onClick={() => setStep('ver')}>Voltar</button></>
          : <><button className="ghost-button" disabled={step === 'rodando'} onClick={close}>Cancelar</button>
            <button className="danger-action" disabled={!previa?.comando || step === 'rodando'} onClick={() => setStep('confirmar')}>{step === 'rodando' ? 'Desinstalando…' : 'Desinstalar'}</button></>}
      </div>
    </div>
  </div>;
}

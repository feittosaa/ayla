import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { errorText, native } from '../services/Desktop';
import { slug, tipos, type Campo, type FichaAberta, type FichaStatus, type Mapa, type Tipo, type Traco, type Valor, type Valores } from '../services/Ficha';
import { CloseIcon, PlusIcon, TrashIcon } from './Icons';

type Target = { root: string; path: string; name: string };
type View = { kind: 'inicio' } | { kind: 'ficha'; primeira: number; nome: string } | { kind: 'mapa' };
/** Resolução das páginas desenhadas pelo núcleo (`ficha_pagina`). */
const DPI = 110;

/** Fichas de RPG de um PDF de cofre. A Ayla acha as fichas e o modelo sozinha; aqui se editam os
 * valores (com prévia) e se ajusta o modelo. O PDF é alterado no mesmo lugar e a versão de antes
 * fica em Configurações › Backups. Usa os mesmos serviços da conversa. */
export default function IrisFicha({ target, onChanged, onClose }: { target: Target; onChanged: () => void; onClose: () => void }) {
  const [status, setStatus] = useState<FichaStatus | null>(null);
  const [view, setView] = useState<View>({ kind: 'inicio' });
  const [error, setError] = useState('');
  const load = useCallback((refazer = false) => native<FichaStatus>('ficha_status', { root: target.root, path: target.path, refazer }).then(setStatus, e => setError(errorText(e))), [target.root, target.path]);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape' && !document.querySelector('.dialog-backdrop')) { event.stopPropagation(); onClose(); } };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [onClose]);
  return <div className="iris-editor iris-ficha" role="dialog" aria-modal="true" aria-label={`Ficha de ${target.name}`} onClick={e => e.stopPropagation()}>
    <header className="iris-viewer-bar">
      <span className="iris-viewer-title"><b>{view.kind === 'mapa' ? 'Modelo da ficha' : view.kind === 'ficha' ? view.nome : 'Fichas de RPG'}</b><small>{target.name}</small></span>
      {view.kind !== 'inicio' && <button className="ghost-button iris-outline-button" onClick={() => { setView({ kind: 'inicio' }); void load(); }}>Voltar</button>}
      <button className="icon-button tip-end" data-tip="Fechar (Esc)" aria-label="Fechar a ficha" onClick={onClose}><CloseIcon size={18} /></button>
    </header>
    {error && <p role="alert" className="app-error iris-ficha-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
    {!status ? <p className="iris-empty" role="status">Procurando as fichas deste PDF…</p>
      : view.kind === 'inicio' ? <Inicio target={target} status={status} onOpen={(primeira, nome) => setView({ kind: 'ficha', primeira, nome })} onMap={() => setView({ kind: 'mapa' })}
          onRedetect={() => { setStatus(null); void load(true); }} />
      : view.kind === 'ficha' ? <Formulario target={target} primeira={view.primeira} onApplied={onChanged} onError={setError} />
      : <EditorMapa target={target} total={status.paginas} onSaved={next => { setStatus(next); setView({ kind: 'inicio' }); }} onError={setError} />}
  </div>;
}

/** Página do PDF desenhada pelo núcleo; `onSize` recebe o tamanho em pixels (para o editor do mapa). */
function Pagina({ target, page, onSize, editing, children }: {
  target: { root: string; path: string }; page: number; onSize?: (s: [number, number]) => void; editing?: boolean; children?: React.ReactNode;
}) {
  const key = `${target.root}|${target.path}|${page}`;
  const [shown, setShown] = useState<{ key: string; src: string } | null>(null);
  useEffect(() => {
    let alive = true;
    void native<string>('ficha_pagina', { root: target.root, path: target.path, pagina: page }).then(src => { if (alive) setShown({ key, src }); }, () => {});
    return () => { alive = false; };
  }, [key, target.root, target.path, page]);
  const src = shown?.key === key ? shown.src : null;
  return <div className={editing ? 'iris-ficha-page editing' : 'iris-ficha-page'}>
    {src ? <img src={src} alt={`Página ${page}`} draggable={false} onLoad={e => onSize?.([e.currentTarget.naturalWidth, e.currentTarget.naturalHeight])} /> : <p role="status">Desenhando a página {page}…</p>}
    {src && children}
  </div>;
}

function Inicio({ target, status, onOpen, onMap, onRedetect }: {
  target: Target; status: FichaStatus; onOpen: (primeira: number, nome: string) => void; onMap: () => void; onRedetect: () => void;
}) {
  const [shown, setShown] = useState(status.fichas[0]?.primeira ?? 1);
  return <div className="iris-editor-body">
    <div className="iris-editor-main iris-ficha-stage"><Pagina target={target} page={shown} /></div>
    <aside className="iris-editor-panel">
      <section>
        <h3>Fichas deste PDF</h3>
        {status.fichas.length ? status.fichas.map(f => <button key={f.primeira} className="iris-ficha-item" onMouseEnter={() => setShown(f.primeira)} onFocus={() => setShown(f.primeira)} onClick={() => onOpen(f.primeira, f.nome)}>
          <b>{f.nome}</b><small>{status.porFicha > 1 ? `Páginas ${f.primeira} a ${f.primeira + status.porFicha - 1}` : `Página ${f.primeira}`}</small></button>)
          : <p className="iris-upscale-note">Não achei fichas neste PDF. Ajuste o modelo abaixo.</p>}
        <p className="iris-upscale-note">Pela conversa também funciona: “adiciona um ponto em Golpear na ficha do {status.fichas[0]?.nome.split(' ')[0] ?? 'personagem'}”. O PDF é alterado no mesmo lugar e a versão de antes fica em Configurações › Backups.</p>
      </section>
      <section>
        <h3>Modelo</h3>
        <p className="iris-upscale-note">{status.origem}</p>
        <button className="ghost-button iris-outline-button" onClick={onMap}>Ajustar o modelo</button>
        <button className="ghost-button iris-outline-button" onClick={onRedetect}>Detectar de novo</button>
      </section>
    </aside>
  </div>;
}

/** Valores da ficha: um controle por campo, prévia e Aplicar (refaz o PDF no mesmo lugar). */
function Formulario({ target, primeira, onApplied, onError }: { target: Target; primeira: number; onApplied: () => void; onError: (e: string) => void }) {
  const [ficha, setFicha] = useState<FichaAberta | null>(null);
  const [valores, setValores] = useState<Valores>({});
  const [previews, setPreviews] = useState<string[] | null>(null);
  const [busy, setBusy] = useState<'' | 'previa' | 'aplicar'>('');
  const [done, setDone] = useState('');
  const [version, setVersion] = useState(0);
  const { root, path } = target;
  useEffect(() => {
    void native<FichaAberta>('ficha_abrir', { root, path, primeira }).then(f => { setFicha(f); setValores(f.valores); }, e => onError(errorText(e)));
  }, [root, path, primeira, onError]);
  const changed = useMemo(() => ficha !== null && JSON.stringify(ficha.valores) !== JSON.stringify(valores), [ficha, valores]);
  const set = (id: string, value: Valor | null) => { setDone(''); setPreviews(null); setValores(prev => { const next = { ...prev }; if (value === null || value === '') delete next[id]; else next[id] = value; return next; }); };
  async function preview() {
    setBusy('previa');
    try { setPreviews(await native<string[]>('ficha_previa', { root, path, primeira, valores })); } catch (e) { onError(errorText(e)); } finally { setBusy(''); }
  }
  async function apply() {
    setBusy('aplicar');
    try {
      const result = await native<{ mudouFora: boolean }>('ficha_aplicar', { root, path, primeira, valores });
      setFicha(f => f && { ...f, valores });
      setPreviews(null); setVersion(v => v + 1);
      setDone(result.mudouFora ? 'Aplicado. O PDF tinha mudado fora da Ayla; a versão nova virou a base.' : 'Aplicado no PDF, no mesmo lugar. A versão de antes está em Configurações › Backups.');
      onApplied();
    } catch (e) { onError(errorText(e)); } finally { setBusy(''); }
  }
  if (!ficha) return <p className="iris-empty" role="status">Abrindo a ficha…</p>;
  const pages = Array.from({ length: ficha.mapa.paginas }, (_, i) => i);
  return <div className="iris-editor-body">
    <div className="iris-editor-main iris-ficha-stage">
      {previews ? previews.map((src, i) => <div key={i} className="iris-ficha-page"><img src={src} alt={`Prévia da página ${i + 1}`} /></div>)
        : pages.map(p => <Pagina key={`${p}-${version}`} target={target} page={ficha.primeira + p} />)}
    </div>
    <aside className="iris-editor-panel iris-ficha-form">
      {pages.map(p => <section key={p}>
        {ficha.mapa.paginas > 1 && <h3>Página {p + 1}</h3>}
        {ficha.mapa.campos.filter(c => c.pagina === p).map(c => <CampoInput key={c.id} campo={c} valor={valores[c.id]} onChange={v => set(c.id, v)} />)}
      </section>)}
      <div className="iris-ficha-actions">
        {done && <p role="status" className="chat-ficha-done">{done}</p>}
        <button className="ghost-button iris-outline-button" disabled={!!busy} onClick={() => void preview()}>{busy === 'previa' ? 'Montando a prévia…' : previews ? 'Prévia atualizada' : 'Ver prévia'}</button>
        <button className="primary-action" disabled={!changed || !!busy} onClick={() => void apply()}>{busy === 'aplicar' ? 'Aplicando…' : 'Aplicar'}</button>
      </div>
    </aside>
  </div>;
}

function CampoInput({ campo, valor, onChange }: { campo: Campo; valor: Valor | undefined; onChange: (v: Valor | null) => void }) {
  if (campo.tipo === 'bolinhas') {
    const n = typeof valor === 'number' ? valor : 0;
    return <div className="iris-ficha-field"><span>{campo.rotulo}</span><div className="iris-ficha-dots" role="group" aria-label={campo.rotulo}>
      {Array.from({ length: campo.maximo }, (_, i) => <button key={i} type="button" aria-label={`${i + 1} de ${campo.maximo}`} aria-pressed={i < n} onClick={() => onChange(i + 1 === n ? i : i + 1)} />)}
    </div></div>;
  }
  if (campo.tipo === 'numero') return <label className="iris-ficha-field"><span>{campo.rotulo}</span>
    <input type="number" value={typeof valor === 'number' ? valor : ''} onChange={e => onChange(e.target.value === '' ? null : Math.trunc(Number(e.target.value)))} /></label>;
  if (campo.tipo === 'linha') return <label className="iris-ficha-field wide"><span>{campo.rotulo}</span><input value={typeof valor === 'string' ? valor : ''} onChange={e => onChange(e.target.value)} /></label>;
  if (campo.tipo === 'bloco') return <label className="iris-ficha-field wide"><span>{campo.rotulo}</span><textarea rows={4} value={typeof valor === 'string' ? valor : ''} onChange={e => onChange(e.target.value)} /></label>;
  const list: Traco[] = Array.isArray(valor) ? valor : [];
  const update = (next: Traco[]) => onChange(next.length ? next : null);
  return <div className="iris-ficha-field wide"><span>{campo.rotulo}</span>
    {list.map((t, i) => <div key={i} className="iris-ficha-traco">
      <input aria-label="Título do traço" value={t.titulo} placeholder="Título" onChange={e => update(list.map((x, j) => j === i ? { ...x, titulo: e.target.value } : x))} />
      <button type="button" className="icon-button" aria-label={`Tirar ${t.titulo || 'traço'}`} onClick={() => update(list.filter((_, j) => j !== i))}><TrashIcon size={14} /></button>
      <textarea aria-label="Descrição do traço" rows={2} value={t.descricao} placeholder="Descrição" onChange={e => update(list.map((x, j) => j === i ? { ...x, descricao: e.target.value } : x))} />
    </div>)}
    <button type="button" className="ghost-button iris-outline-button" onClick={() => update([...list, { titulo: '', descricao: '' }])}><PlusIcon size={14} /> Traço</button>
  </div>;
}

type Drag = { id: string; mode: 'move' | 'resize'; x: number; y: number; caixa: Campo['caixa']; width: number; pageWidth: number };

/** Mapa do modelo: a Ayla propõe os campos pelos rótulos; aqui se confere o nome, o tipo e a
 * posição de cada um (arrastar move; o canto redimensiona). */
function EditorMapa({ target, total, onSaved, onError }: { target: Target; total: number; onSaved: (status: FichaStatus) => void; onError: (e: string) => void }) {
  const [mapa, setMapa] = useState<Mapa>({ nome: target.name.replace(/\.pdf$/i, '') || 'Ficha', paginas: 1, campos: [] });
  const [loaded, setLoaded] = useState(false);
  const [primeira, setPrimeira] = useState(1);
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState<'' | 'salvar'>('');
  const [size, setSize] = useState<[number, number] | null>(null);
  const drag = useRef<Drag | null>(null);
  useEffect(() => {
    void native<{ mapa: Mapa | null; primeira: number }>('ficha_modelo', { root: target.root, path: target.path })
      .then(r => { if (r.mapa) setMapa(r.mapa); setPrimeira(r.primeira); setLoaded(true); }, e => onError(errorText(e)));
  }, [target.root, target.path, onError]);
  async function salvar() {
    setBusy('salvar');
    try { onSaved(await native<FichaStatus>('ficha_modelo_salvar', { root: target.root, path: target.path, mapa })); } catch (e) { onError(errorText(e)); } finally { setBusy(''); }
  }
  const update = (id: string, change: Partial<Campo> | ((c: Campo) => Campo)) => setMapa(m => ({ ...m, campos: m.campos.map(c => c.id !== id ? c : typeof change === 'function' ? change(c) : { ...c, ...change } as Campo) }));
  const current = mapa.campos.find(c => c.id === selected) ?? null;
  // Largura da página em pontos, pela imagem (DPI conhecido).
  const pageWidth = size ? size[0] * 72 / DPI : 0;
  const pageHeight = size ? size[1] * 72 / DPI : 0;
  function startDrag(event: ReactPointerEvent, campo: Campo, mode: Drag['mode']) {
    event.stopPropagation();
    setSelected(campo.id);
    const frame = (event.currentTarget as HTMLElement).closest('.iris-ficha-page')?.getBoundingClientRect();
    if (!frame || !pageWidth) return;
    drag.current = { id: campo.id, mode, x: event.clientX, y: event.clientY, caixa: campo.caixa, width: frame.width, pageWidth };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }
  function moveDrag(event: ReactPointerEvent) {
    const d = drag.current;
    if (!d) return;
    const k = d.pageWidth / d.width;
    const dx = (event.clientX - d.x) * k, dy = (event.clientY - d.y) * k;
    const [x0, y0, x1, y1] = d.caixa;
    const round = (v: number) => Math.round(v * 10) / 10;
    update(d.id, { caixa: d.mode === 'move' ? [round(x0 + dx), round(y0 + dy), round(x1 + dx), round(y1 + dy)] : [x0, y0, round(Math.max(x0 + 4, x1 + dx)), round(Math.max(y0 + 4, y1 + dy))] });
  }
  function addField() {
    const base = slug('Campo novo');
    let id = base; let n = 2;
    while (mapa.campos.some(c => c.id === id)) id = `${base}-${n++}`;
    const cx = (pageWidth || 595) / 2, cy = (pageHeight || 842) / 2;
    setMapa(m => ({ ...m, campos: [...m.campos, { id, rotulo: 'Campo novo', pagina: page, caixa: [cx - 40, cy - 10, cx + 40, cy + 10], tipo: 'linha' }] }));
    setSelected(id);
  }
  function setTipo(campo: Campo, tipo: Tipo['tipo']) {
    update(campo.id, c => ({ ...c, tipo, ...(tipo === 'bolinhas' ? { maximo: 'maximo' in c ? c.maximo : 3 } : {}) } as Campo));
  }
  if (!loaded) return <p className="iris-empty" role="status">Abrindo o modelo…</p>;
  return <div className="iris-editor-body">
    <div className="iris-editor-main iris-ficha-stage">
      {mapa.paginas > 1 && <div className="iris-ficha-tabs" role="tablist">{Array.from({ length: mapa.paginas }, (_, i) => <button key={i} role="tab" aria-selected={page === i} onClick={() => setPage(i)}>Página {i + 1}</button>)}</div>}
      <Pagina target={target} page={primeira + page} onSize={setSize} editing>
        {pageWidth > 0 && mapa.campos.filter(c => c.pagina === page).map(c => {
          const [x0, y0, x1, y1] = c.caixa;
          return <div key={c.id} className={c.id === selected ? 'iris-ficha-box selected' : 'iris-ficha-box'} title={c.rotulo}
            style={{ left: `${x0 / pageWidth * 100}%`, top: `${y0 / pageHeight * 100}%`, width: `${(x1 - x0) / pageWidth * 100}%`, height: `${(y1 - y0) / pageHeight * 100}%` }}
            onPointerDown={e => startDrag(e, c, 'move')} onPointerMove={moveDrag} onPointerUp={() => { drag.current = null; }}>
            <small>{c.rotulo}</small>
            <span className="iris-ficha-resize" onPointerDown={e => startDrag(e, c, 'resize')} onPointerMove={moveDrag} onPointerUp={() => { drag.current = null; }} />
          </div>;
        })}
      </Pagina>
    </div>
    <aside className="iris-editor-panel iris-ficha-form">
      <section>
        <h3>Modelo</h3>
        <p className="iris-upscale-note">A Ayla montou o modelo sozinha; aqui você confere os campos. Fica guardado na Ayla, não no cofre.</p>
        <div className="iris-editor-row">
          <label>Páginas por ficha<input type="number" min={1} max={Math.min(20, total)} value={mapa.paginas} onChange={e => setMapa(m => ({ ...m, paginas: Math.max(1, Math.min(20, total, Number(e.target.value) || 1)) }))} /></label>
          <label>Mostrar a ficha da página<input type="number" min={1} max={total} value={primeira} onChange={e => setPrimeira(Math.max(1, Math.min(total, Number(e.target.value) || 1)))} /></label>
        </div>
      </section>
      {current ? <section>
        <h3>Campo selecionado</h3>
        <label>Nome<input value={current.rotulo} onChange={e => update(current.id, { rotulo: e.target.value })} /></label>
        <label>Tipo<select value={current.tipo} onChange={e => setTipo(current, e.target.value as Tipo['tipo'])}>{tipos.map(t => <option key={t.id} value={t.id}>{t.label}</option>)}</select></label>
        {current.tipo === 'bolinhas' && <label>Quantas bolinhas<input type="number" min={1} max={20} value={current.maximo} onChange={e => update(current.id, c => ({ ...c, maximo: Math.max(1, Math.min(20, Number(e.target.value) || 1)) } as Campo))} /></label>}
        <small className="iris-upscale-note">Chave: {current.id}. Arraste a caixa para mover; o canto de baixo muda o tamanho.</small>
        <button className="ghost-button iris-outline-button" onClick={() => { setMapa(m => ({ ...m, campos: m.campos.filter(c => c.id !== current.id) })); setSelected(null); }}><TrashIcon size={14} /> Tirar este campo</button>
      </section> : <section><p className="iris-upscale-note">Clique numa caixa para conferir o nome e o tipo. Pontos com bolinhas, números (como Golpear), texto curto (Nome), texto longo (Técnicas) e traços (título e descrição).</p></section>}
      <section>
        <h3>Campos ({mapa.campos.length})</h3>
        <div className="iris-ficha-list">{mapa.campos.map(c => <button key={c.id} aria-pressed={c.id === selected} onClick={() => { setSelected(c.id); setPage(c.pagina); }}>
          <span>{c.rotulo}</span><small>{tipos.find(t => t.id === c.tipo)?.label}{c.tipo === 'bolinhas' ? ` · ${c.maximo}` : ''}{mapa.paginas > 1 ? ` · p${c.pagina + 1}` : ''}</small></button>)}</div>
        <button className="ghost-button iris-outline-button" onClick={addField}><PlusIcon size={14} /> Campo</button>
      </section>
      <div className="iris-ficha-actions"><button className="primary-action" disabled={!!busy || !mapa.nome.trim() || !mapa.campos.length} onClick={() => void salvar()}>{busy === 'salvar' ? 'Salvando…' : 'Salvar o modelo'}</button></div>
    </aside>
  </div>;
}

import { Component, lazy, Suspense, useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { errorText, formatBytes, native, nativeTask, taskId } from '../services/Desktop';
import { OPEN_DESIGN_EVENT, MAX_SIDE, MIN_SIDE, clampSide, defaultName, draftOf, formatOf, formats, fromPx, takePendingDesign, templates, toPx, type DesignSummary, type Format, type Template, type Unit } from '../services/Design';
import { loadPackFonts, resetPackFonts } from '../services/DesignFonts';
import { FileIcon, PlusIcon, TrashIcon } from './Icons';
import { documentoDe } from '../services/DesignDocument';
import { mediaUrl, type MediaRef } from '../services/Media';
import type { Opening } from './IrisDesignEditor';

const IrisDesignEditor = lazy(() => import('./IrisDesignEditor'));
/** Um erro no editor fecha só o editor (com o aviso), sem derrubar a Ayla. */
class EditorGuard extends Component<{ onError: (message: string) => void; children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(error: unknown) { this.props.onError(`O editor de designs falhou: ${errorText(error)}`); }
  render() { return this.state.failed ? null : this.props.children; }
}

type LibraryItem = { key: string; name: string; place: string; kind: string; source: string; file?: string; root?: string | null; path?: string };
const EDITABLE = /\.(pdf|png|jpe?g|jfif|webp)$/i;

/** Escolher um PDF ou imagem da Athena ou da Iris para editar por cima. */
function DocumentPicker({ close, choose }: { close: () => void; choose: (item: LibraryItem) => Promise<void> }) {
  const [items, setItems] = useState<LibraryItem[] | null>(null);
  const [query, setQuery] = useState('');
  const [kind, setKind] = useState<'' | 'pdf' | 'image'>('');
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  useEffect(() => { void native<{ items: LibraryItem[] }>('iris_library').then(r => setItems(r.items.filter(i => (i.kind === 'pdf' || i.kind === 'image') && EDITABLE.test(i.name)))).catch(e => setError(errorText(e))); }, []);
  const q = query.trim().toLowerCase();
  const shown = (items ?? []).filter(i => (!kind || i.kind === kind) && (!q || `${i.name} ${i.place}`.toLowerCase().includes(q))).slice(0, 120);
  return <div className="dialog-backdrop" onClick={() => !busy && close()}>
    <div className="dialog design-picker" role="dialog" aria-modal="true" aria-labelledby="design-picker-title" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape' && !busy) close(); }}>
      <h2 id="design-picker-title">Editar um PDF ou imagem</h2>
      <p className="design-hint">Da Athena e da Iris. O que você adicionar fica por cima; "Salvar no arquivo" grava no próprio arquivo, com uma cópia do original nos backups.</p>
      <div className="design-picker-bar">
        <input className="organ-search" type="search" autoFocus value={query} placeholder="Buscar pelo nome ou pasta" aria-label="Buscar arquivo" onChange={e => setQuery(e.target.value)} />
        <div className="design-sources" role="tablist" aria-label="Tipo">
          {([['', 'Todos'], ['pdf', 'PDFs'], ['image', 'Imagens']] as const).map(([k, l]) => <button key={k} role="tab" aria-selected={kind === k} className={kind === k ? 'active' : ''} onClick={() => setKind(k)}>{l}</button>)}
        </div>
      </div>
      {error && <p role="alert" className="app-error">{error}</p>}
      {items === null && !error ? <p className="design-hint" role="status">Reunindo os arquivos…</p>
        : !shown.length ? <p className="design-hint">Nenhum arquivo encontrado.</p>
        : <ul className="design-picker-list">{shown.map(item => <li key={item.key}>
            <button disabled={Boolean(busy)} onClick={async () => { setBusy(item.key); setError(''); try { await choose(item); } catch (e) { setError(errorText(e)); setBusy(''); } }}>
              {item.kind === 'image' ? <img src={mediaUrl((item.file ? { file: item.file } : { root: item.root ?? '', path: item.path ?? '' }) as MediaRef, true)} alt="" loading="lazy" /> : <span className="design-picker-pdf"><FileIcon size={22} /><small>PDF</small></span>}
              <span><b>{item.name}</b><small>{item.place}</small></span>
              {busy === item.key && <small role="status">Abrindo…</small>}
            </button>
          </li>)}</ul>}
      <div className="row-actions dialog-actions"><button className="ghost-button" disabled={Boolean(busy)} onClick={close}>Cancelar</button></div>
    </div>
  </div>;
}

/** Tamanho livre: largura e altura em px, cm ou mm (a unidade escolhida fica guardada). */
function CustomSize({ close, create }: { close: () => void; create: (width: number, height: number) => void }) {
  const [unit, setUnit] = useState<Unit>(() => (['px', 'cm', 'mm'] as const).find(u => u === localStorage.getItem('ayla-design-unidade')) ?? 'px');
  const [size, setSize] = useState(() => ({ width: fromPx(1200, unit), height: fromPx(800, unit) }));
  const px = { width: Math.round(toPx(size.width, unit)), height: Math.round(toPx(size.height, unit)) };
  const valid = [px.width, px.height].every(v => Number.isFinite(v) && v >= MIN_SIDE && v <= MAX_SIDE);
  const final = { width: clampSide(px.width), height: clampSide(px.height) };
  function changeUnit(next: Unit) {
    setSize({ width: fromPx(toPx(size.width, unit), next), height: fromPx(toPx(size.height, unit), next) });
    setUnit(next); localStorage.setItem('ayla-design-unidade', next);
  }
  const step = unit === 'px' ? 1 : 0.1;
  return <div className="dialog-backdrop" onPointerDown={e => { if (e.target === e.currentTarget) close(); }}>
    <form className="dialog design-size-dialog" role="dialog" aria-modal="true" aria-labelledby="design-size-title" onKeyDown={e => { if (e.key === 'Escape') close(); }}
      onSubmit={e => { e.preventDefault(); if (valid) create(final.width, final.height); }}>
      <h2 id="design-size-title">Tamanho livre</h2>
      <div className="design-size-fields">
        <label className="design-field">Largura <input type="number" autoFocus min={0} step={step} value={size.width} onChange={e => setSize(s => ({ ...s, width: Number(e.target.value) }))} /></label>
        <span aria-hidden="true">×</span>
        <label className="design-field">Altura <input type="number" min={0} step={step} value={size.height} onChange={e => setSize(s => ({ ...s, height: Number(e.target.value) }))} /></label>
        <label className="design-field">Unidade <select value={unit} onChange={e => changeUnit(e.target.value as Unit)}>
          <option value="px">px</option><option value="cm">cm</option><option value="mm">mm</option>
        </select></label>
      </div>
      <div className="design-size-preview"><FormatShape format={final} /></div>
      <p className="design-hint" role="status">{valid
        ? `${final.width} × ${final.height} px${unit === 'px' ? '' : ` (${unit}, a 96 px por polegada: no PDF sai com essa medida)`}.`
        : `Cada lado deve ter de ${fromPx(MIN_SIDE, unit)} a ${fromPx(MAX_SIDE, unit)} ${unit}.`}</p>
      <div className="row-actions dialog-actions">
        <button type="button" className="ghost-button" onClick={close}>Cancelar</button>
        <button type="submit" className="primary-action" disabled={!valid}><PlusIcon size={15} /> Criar</button>
      </div>
    </form>
  </div>;
}

const day = (seconds: number) => new Date(seconds * 1000).toLocaleDateString('pt-BR', { day: '2-digit', month: 'short', year: 'numeric' });

/** Moldura proporcional de um formato (sem o canvas). */
function FormatShape({ format }: { format: Pick<Format, 'width' | 'height'> }) {
  const k = 54 / Math.max(format.width, format.height);
  return <span className="design-format-shape" style={{ width: format.width * k, height: format.height * k }} />;
}

/** Designs da Iris: começar por um formato, um tamanho livre ou um modelo; os guardados; e o
 * pacote opcional de recursos (fontes e ícones). O editor abre por cima, como o visualizador. */
export default function IrisDesigns({ active, onCount, onExported }: { active: boolean; onCount: (count: number) => void; onExported: () => void }) {
  const [designs, setDesigns] = useState<DesignSummary[] | null>(null);
  const [packInstalled, setPackInstalled] = useState(false);
  const [packBytes, setPackBytes] = useState(0);
  const [progress, setProgress] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [opening, setOpening] = useState<Opening | null>(null);
  const [sizing, setSizing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [previews, setPreviews] = useState<Record<string, string>>({});
  const [picking, setPicking] = useState(false);
  const job = useRef<string | null>(null);

  const load = useCallback(() => native<{ designs: DesignSummary[]; pack: boolean; packBytes: number }>('design_list').then(result => {
    setDesigns(result.designs); setPackInstalled(result.pack); setPackBytes(result.packBytes); onCount(result.designs.length);
  }, e => setError(errorText(e))), [onCount]);
  useEffect(() => { if (active) void load(); }, [active, load]);
  useEffect(() => () => { if (job.current) void native('cancel_job', { requestId: job.current }); }, []);

  // Miniaturas dos modelos, desenhadas pelo próprio Fabric (com as fontes do pacote, se houver).
  useEffect(() => {
    if (!active) return;
    let alive = true;
    void Promise.all([import('../services/DesignCanvas'), loadPackFonts()]).then(([canvas]) => {
      if (!alive) return;
      const out: Record<string, string> = {};
      for (const template of templates) {
        try { out[template.id] = canvas.templatePreview(template); } catch { /* fica a moldura */ }
      }
      setPreviews(out);
    });
    return () => { alive = false; };
  }, [active, packInstalled]);

  async function installPack() {
    const requestId = taskId(); job.current = requestId;
    setProgress('Preparando…'); setError('');
    try {
      await nativeTask('install_runtime', { id: 'design', requestId }, event => {
        if (event.type === 'download') setProgress(`Baixando ${formatBytes(event.received || 0)} de ${formatBytes(event.total || 0)}`);
      });
      resetPackFonts();
      await load();
    } catch (e) { setError(errorText(e)); }
    finally { job.current = null; setProgress(null); }
  }
  async function cancelPack() { if (job.current) await native('cancel_job', { requestId: job.current }); }

  function start(format: Pick<Format, 'label' | 'width' | 'height'>, template?: Template) {
    setNotice('');
    setOpening({ name: defaultName(template?.name ?? format.label), width: format.width, height: format.height, template });
  }
  async function open(design: Pick<DesignSummary, 'id'>) {
    setNotice(''); setError('');
    try {
      let file = await native<DesignSummary & { data: string }>('design_read', { id: design.id });
      // Rascunho criado pela Ayla na conversa e ainda não montado: monta e salva antes de abrir.
      if (draftOf(file.data)) {
        await (await import('../services/DesignCanvas')).ensureDesign(design.id);
        file = await native<DesignSummary & { data: string }>('design_read', { id: design.id });
      }
      setOpening({ id: file.id, name: file.name, width: file.width, height: file.height, data: file.data });
    } catch (e) { setError(errorText(e)); }
  }
  // "Abrir no editor" na conversa: abre o design pedido assim que Designs aparece.
  useEffect(() => {
    if (!active) return;
    const take = () => { const id = takePendingDesign(); if (id) { setOpening(null); void open({ id }); } };
    take();
    window.addEventListener(OPEN_DESIGN_EVENT, take);
    return () => window.removeEventListener(OPEN_DESIGN_EVENT, take);
  }, [active]);
  // Rascunhos da conversa ainda sem prévia (criados com a conversa fechada): monta ao listar.
  useEffect(() => {
    const drafts = designs?.filter(d => !d.preview) ?? [];
    if (!active || !drafts.length) return;
    let alive = true;
    void (async () => {
      const canvas = await import('../services/DesignCanvas');
      let changed = false;
      for (const d of drafts) { try { if (await canvas.ensureDesign(d.id)) changed = true; } catch { /* fica sem prévia */ } }
      if (alive && changed) void load();
    })();
    return () => { alive = false; };
  }, [active, designs, load]);
  async function remove(id: string) {
    try { await native('design_delete', { id }); setConfirmDelete(null); await load(); }
    catch (e) { setError(errorText(e)); }
  }

  const pack = { installed: packInstalled, bytes: packBytes, progress, install: () => void installPack() };
  return <div className="designs">
    {error && <p role="alert" className="app-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
    {notice && <p role="status" className="design-notice">{notice} <button className="link-button" onClick={() => setNotice('')}>OK</button></p>}
    <section aria-label="Editar um arquivo">
      <h2>Editar um arquivo</h2>
      <button className="design-format design-open-file" onClick={() => setPicking(true)}>
        <span className="design-picker-pdf"><FileIcon size={26} /></span><b>Abrir um PDF ou imagem</b><small>Da Athena ou da Iris: escreva, marque e adicione imagens por cima, e grave no próprio arquivo</small>
      </button>
    </section>
    {sizing && <CustomSize close={() => setSizing(false)} create={(w, h) => { setSizing(false); start({ label: 'Tamanho livre', width: w, height: h }); }} />}
    {picking && <DocumentPicker close={() => setPicking(false)} choose={async item => {
      const documento = await documentoDe(item);
      setPicking(false); setNotice('');
      setOpening({ name: documento.nome.replace(/\.[^.]+$/, ''), width: 1200, height: 800, documento });
    }} />}
    <section aria-label="Novo design">
      <h2>Criar um design</h2>
      <div className="design-formats">
        {formats.map(format => <button key={format.id} className="design-format" onClick={() => start(format)}>
          <FormatShape format={format} /><b>{format.label}</b><small>{format.width} × {format.height} px · {format.hint}</small>
        </button>)}
        <button className="design-format custom" onClick={() => setSizing(true)}>
          <span className="design-format-shape custom-shape"><PlusIcon size={20} /></span><b>Tamanho livre</b><small>Em px, cm ou mm</small>
        </button>
      </div>
    </section>
    <section aria-label="Modelos">
      <h2>Modelos</h2>
      <div className="design-templates">
        {templates.map(template => {
          const format = formatOf(template.format)!;
          return <button key={template.id} className="design-template" onClick={() => start(format, template)}>
            <span className="design-template-preview" style={{ aspectRatio: `${format.width} / ${format.height}` }}>{previews[template.id] && <img src={previews[template.id]} alt="" />}</span>
            <b>{template.name}</b><small>{format.label}</small>
          </button>;
        })}
      </div>
      {!packInstalled && <div className="design-pack inline">
        <p><b>Pacote de recursos (opcional):</b> 12 fontes livres usadas nos modelos e mais de 2.000 ícones (Lucide). {formatBytes(packBytes)}, baixado uma vez. Sem ele, os textos usam as fontes do sistema.</p>
        {progress ? <><span role="status">{progress}</span><button className="ghost-button" onClick={() => void cancelPack()}>Interromper</button></> : <button className="primary-action" onClick={() => void installPack()}>Baixar pacote</button>}
      </div>}
    </section>
    <section aria-label="Seus designs">
      <h2>Seus designs</h2>
      {designs === null ? <p className="iris-empty" role="status">Carregando…</p>
        : !designs.length ? <p className="iris-empty">Nenhum design ainda. Escolha um formato ou um modelo acima.</p>
        : <div className="design-saved">{designs.map(design => <div key={design.id} className="design-card">
            <button className="design-card-open" title={`Abrir ${design.name}`} onClick={() => void open(design)}>
              <span className="design-template-preview" style={{ aspectRatio: `${design.width} / ${design.height}` }}>{design.preview && <img src={design.preview} alt="" />}</span>
              <b>{design.name}</b><small>{design.width} × {design.height} px · {day(design.modified)}</small>
            </button>
            {confirmDelete === design.id
              ? <span className="design-card-confirm" role="group" aria-label="Confirmar exclusão">
                  <button className="ghost-button" autoFocus onClick={() => setConfirmDelete(null)}>Cancelar</button>
                  <button className="ghost-button danger" onClick={() => void remove(design.id)}>Mover para a lixeira</button>
                </span>
              : <button className="icon-button design-card-delete" data-tip="Excluir design" aria-label={`Excluir ${design.name}`} onClick={() => setConfirmDelete(design.id)}><TrashIcon size={15} /></button>}
          </div>)}</div>}
    </section>
    {opening && <EditorGuard key={opening.id ?? opening.name} onError={message => { setError(message); setOpening(null); }}><Suspense fallback={<div className="design-editor" aria-busy="true"><p className="iris-empty" role="status">Abrindo o editor…</p></div>}>
      <IrisDesignEditor key={opening.id ?? opening.name} opening={opening} pack={pack}
        onSaved={() => void load()} onExported={message => { setNotice(message); onExported(); }} onClose={() => { setOpening(null); void load(); }} />
    </Suspense></EditorGuard>}
  </div>;
}

import { Component, lazy, Suspense, useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { errorText, formatBytes, native, nativeTask, taskId } from '../services/Desktop';
import { OPEN_DESIGN_EVENT, clampSide, defaultName, draftOf, formatOf, formats, takePendingDesign, templates, type DesignSummary, type Format, type Template } from '../services/Design';
import { loadPackFonts, resetPackFonts } from '../services/DesignFonts';
import { PlusIcon, TrashIcon } from './Icons';
import type { Opening } from './IrisDesignEditor';

const IrisDesignEditor = lazy(() => import('./IrisDesignEditor'));
/** Um erro no editor fecha só o editor (com o aviso), sem derrubar a Ayla. */
class EditorGuard extends Component<{ onError: (message: string) => void; children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(error: unknown) { this.props.onError(`O editor de designs falhou: ${errorText(error)}`); }
  render() { return this.state.failed ? null : this.props.children; }
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
  const [custom, setCustom] = useState({ width: 1200, height: 800 });
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [previews, setPreviews] = useState<Record<string, string>>({});
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
    <section aria-label="Novo design">
      <h2>Criar um design</h2>
      <div className="design-formats">
        {formats.map(format => <button key={format.id} className="design-format" onClick={() => start(format)}>
          <FormatShape format={format} /><b>{format.label}</b><small>{format.width} × {format.height} px · {format.hint}</small>
        </button>)}
        <form className="design-format custom" onSubmit={e => { e.preventDefault(); start({ label: 'Tamanho livre', width: clampSide(custom.width), height: clampSide(custom.height) }); }}>
          <FormatShape format={{ width: clampSide(custom.width), height: clampSide(custom.height) }} /><b>Tamanho livre</b>
          <span className="design-custom">
            <input type="number" min={16} max={8000} value={custom.width} aria-label="Largura em px" onChange={e => setCustom(c => ({ ...c, width: Number(e.target.value) }))} />×
            <input type="number" min={16} max={8000} value={custom.height} aria-label="Altura em px" onChange={e => setCustom(c => ({ ...c, height: Number(e.target.value) }))} />px
          </span>
          <button className="ghost-button" type="submit"><PlusIcon size={15} /> Criar</button>
        </form>
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

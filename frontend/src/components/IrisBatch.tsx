import { useEffect, useMemo, useState } from 'react';
import { errorText, native } from '../services/Desktop';
import IrisUpscale, { type BatchReport, type BatchTarget } from './IrisUpscale';
import { CloseIcon } from './Icons';

/** Item escolhido na grade da Iris. */
export type PickedItem = { key: string; name: string; kind: 'image' | 'video' | 'pdf'; source: string; file?: string; root?: string | null; path?: string };
/** Pasta escolhida na lateral (`path` vazio = a pasta principal). */
export type PickedFolder = { root: string; path: string; name: string; athena: boolean };
type Destination = { id: string; name: string; kind: 'computer' | 'athena'; path: string; folders: string[] };
type Report = { done: number; skipped: { name: string; reason: string }[] };
type Result = { title: string; detail?: string; skipped: Report['skipped'] };

const target = (item: PickedItem): BatchTarget => item.file ? { file: item.file } : { root: item.root, path: item.path };
const folderTarget = ({ root, path }: PickedFolder) => ({ root, path });
const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
/** Pode passar por upscale (o núcleo também confere). */
const upscalable = (item: PickedItem) => item.kind === 'image' && !/\.(gif|svg)$/i.test(item.name);

/** "3 imagens, 1 vídeo e 2 pastas". */
function describe(items: PickedItem[], folders: PickedFolder[]) {
  const parts = [
    [items.filter(i => i.kind === 'image').length, 'imagem', 'imagens'],
    [items.filter(i => i.kind === 'video').length, 'vídeo', 'vídeos'],
    [items.filter(i => i.kind === 'pdf').length, 'PDF', 'PDFs'],
    [folders.length, 'pasta', 'pastas'],
  ].filter(([n]) => n).map(([n, one, many]) => count(n as number, one as string, many as string));
  return parts.length > 1 ? `${parts.slice(0, -1).join(', ')} e ${parts[parts.length - 1]}` : parts[0] ?? 'Nada';
}

/** Concordância: imagens e pastas são femininas; com vídeo ou PDF no meio, masculino. */
function selected(items: PickedItem[], folders: PickedFolder[]) {
  const feminine = items.every(i => i.kind === 'image');
  return `selecionad${feminine ? 'a' : 'o'}${items.length + folders.length > 1 ? 's' : ''}`;
}

/** Barra das ações em lote da Iris (upscale, mover e lixeira) e os diálogos de cada uma. */
/** `onChanged`: a ação terminou (limpar a seleção e recarregar); o relatório continua aberto até o OK,
 * que chama `onClear` (sai da seleção). */
export default function IrisBatch({ items, folders, onClear, onChanged }: {
  items: PickedItem[]; folders: PickedFolder[]; onClear: () => void; onChanged: () => void;
}) {
  const [dialog, setDialog] = useState<'upscale' | 'move' | 'trash' | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const label = describe(items, folders);
  const empty = !items.length && !folders.length;
  const toUpscale = items.filter(upscalable);
  const canMove = items.some(i => !i.file) || folders.some(f => f.path);
  const canTrash = items.length > 0 || folders.some(f => f.path);
  function finish(next: Result) {
    setDialog(null);
    setResult(next);
    onChanged();
  }
  function afterUpscale(report: BatchReport) {
    const names = (list: string[]) => list.map(f => `“${f}”`).join(', ');
    const where = [
      report.folders.length ? `Salvas em ${names(report.folders)}, ao lado da pasta de origem.` : '',
      report.connected.length ? `${names(report.connected)} ${report.connected.length === 1 ? 'foi conectada' : 'foram conectadas'} na Iris, em Sistema.` : '',
    ].filter(Boolean).join(' ') || undefined;
    finish({
      title: report.interrupted ? `Upscale interrompido: ${report.done} de ${report.total} prontas` : `${report.done} de ${count(report.total, 'imagem pronta', 'imagens prontas')}`,
      detail: where, skipped: report.skipped,
    });
  }
  return <>
    <div className="iris-batch-bar" role="toolbar" aria-label="Itens selecionados">
      {empty ? <span className="iris-batch-hint">Clique nas imagens ou nas pastas da lateral para selecionar. Shift+clique pega um intervalo; Ctrl+A, tudo o que está na tela.</span>
        : <span className="iris-batch-count">{label} {selected(items, folders)}</span>}
      <div className="iris-batch-actions">
        <button className="ghost-button" disabled={!toUpscale.length && !folders.length} onClick={() => setDialog('upscale')}>Upscale</button>
        <button className="ghost-button" disabled={!canMove} onClick={() => setDialog('move')}>Mover</button>
        <button className="ghost-button iris-batch-danger" disabled={!canTrash} onClick={() => setDialog('trash')}>Excluir</button>
        <button className="icon-button tip-end" data-tip="Sair da seleção (Esc)" aria-label="Sair da seleção" onClick={onClear}><CloseIcon size={16} /></button>
      </div>
    </div>
    {dialog === 'upscale' && <div className="dialog-backdrop iris-batch-upscale">
      <IrisUpscale batch={{ items: toUpscale.map(target), folders: folders.map(folderTarget), label: describe(toUpscale, folders) }}
        onBatchDone={afterUpscale} onClose={() => setDialog(null)} />
    </div>}
    {dialog === 'move' && <MoveDialog items={items} folders={folders} onClose={() => setDialog(null)}
      onDone={(report, place) => finish({ title: `${count(report.done, 'item movido', 'itens movidos')} para “${place}”`, skipped: report.skipped })} />}
    {dialog === 'trash' && <TrashDialog items={items} folders={folders} label={label} onClose={() => setDialog(null)}
      onDone={report => finish({ title: `${count(report.done, 'item foi', 'itens foram')} para a lixeira`, detail: report.done ? 'Dá para recuperar pela lixeira do sistema.' : undefined, skipped: report.skipped })} />}
    {result && <div className="dialog-backdrop" onClick={() => { setResult(null); onClear(); }}>
      <div className="dialog iris-batch-result" role="dialog" aria-modal="true" aria-labelledby="iris-batch-result" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape') { setResult(null); onClear(); } }}>
        <h2 id="iris-batch-result">{result.title}</h2>
        {result.detail && <p>{result.detail}</p>}
        {result.skipped.length > 0 && <>
          <p>{count(result.skipped.length, 'ficou de fora', 'ficaram de fora')}:</p>
          <ul className="iris-batch-skipped">{result.skipped.map((s, i) => <li key={i}><b>{s.name}</b> — {s.reason}</li>)}</ul>
        </>}
        <div className="row-actions"><button className="primary-action" autoFocus onClick={() => { setResult(null); onClear(); }}>OK</button></div>
      </div>
    </div>}
  </>;
}

function TrashDialog({ items, folders, label, onDone, onClose }: {
  items: PickedItem[]; folders: PickedFolder[]; label: string; onDone: (report: Report) => void; onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  async function trash() {
    setBusy(true); setError('');
    try { onDone(await native<Report>('iris_trash_batch', { items: items.map(target), folders: folders.map(folderTarget) })); }
    catch (e) { setError(errorText(e)); setBusy(false); }
  }
  return <div className="dialog-backdrop" onClick={() => { if (!busy) onClose(); }}>
    <div className="dialog" role="alertdialog" aria-modal="true" aria-labelledby="iris-batch-trash" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape' && !busy) onClose(); }}>
      <h2 id="iris-batch-trash">Mover para a lixeira?</h2>
      <p>{label} {items.length + folders.length === 1 ? 'vai' : 'vão'} para a lixeira do sistema; dá para recuperar de lá.</p>
      {folders.some(f => f.path) && <p>{folders.filter(f => f.path).length === 1 ? 'A pasta vai inteira' : 'As pastas vão inteiras'}, com tudo o que tem dentro, inclusive arquivos que a Iris não mostra (como notas da Athena).</p>}
      {folders.some(f => !f.path) && <p>Pastas principais não vão para a lixeira; para tirá-las da Iris, use Desconectar.</p>}
      {items.some(i => i.file) && <p>Anexos do chat e imagens da Ayla também saem da Ayla.</p>}
      {error && <p role="alert" className="iris-upscale-error">{error}</p>}
      <div className="row-actions">
        <button className="danger-action" disabled={busy} onClick={() => void trash()}>{busy ? 'Movendo…' : 'Mover para a lixeira'}</button>
        <button className="ghost-button" autoFocus disabled={busy} onClick={onClose}>Cancelar</button>
      </div>
    </div>
  </div>;
}

function MoveDialog({ items, folders, onDone, onClose }: {
  items: PickedItem[]; folders: PickedFolder[]; onDone: (report: Report, place: string) => void; onClose: () => void;
}) {
  const [roots, setRoots] = useState<Destination[] | null>(null);
  const [filter, setFilter] = useState('');
  const [chosen, setChosen] = useState<{ root: string; path: string; label: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { native<Destination[]>('iris_destinations').then(setRoots, e => setError(errorText(e))); }, []);
  const rows = useMemo(() => {
    const words = filter.trim().toLowerCase();
    return (roots ?? []).flatMap(root => [
      { root: root.id, path: '', name: root.name, full: root.name, depth: 0, athena: root.kind === 'athena' },
      ...root.folders.map(path => ({ root: root.id, path, name: path.split('/').pop() ?? path, full: `${root.name} / ${path.replaceAll('/', ' / ')}`, depth: path.split('/').length, athena: root.kind === 'athena' })),
    ]).filter(row => !words || row.full.toLowerCase().includes(words));
  }, [roots, filter]);
  // Arquivos e pastas que saem de um cofre: os links das notas para eles deixam de achá-los.
  const leavesVault = chosen !== null && [...items.filter(i => !i.file && i.source === 'athena'), ...folders.filter(f => f.athena && f.path)].some(x => x.root !== chosen.root);
  async function move() {
    if (!chosen) return;
    setBusy(true); setError('');
    try { onDone(await native<Report>('iris_move_batch', { items: items.map(target), folders: folders.map(folderTarget), root: chosen.root, path: chosen.path }), chosen.label); }
    catch (e) { setError(errorText(e)); setBusy(false); }
  }
  return <div className="dialog-backdrop" onClick={() => { if (!busy) onClose(); }}>
    <div className="dialog iris-move-dialog" role="dialog" aria-modal="true" aria-labelledby="iris-batch-move" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape' && !busy) onClose(); }}>
      <h2 id="iris-batch-move">Mover {describe(items, folders).toLowerCase()} para…</h2>
      <input type="search" className="organ-search" autoFocus placeholder="Filtrar pastas" aria-label="Filtrar pastas" value={filter} onChange={e => setFilter(e.target.value)} />
      <div className="iris-move-list" role="listbox" aria-label="Pastas de destino">
        {!roots && !error ? <p role="status">Listando as pastas…</p>
          : !rows.length ? <p>Nenhuma pasta com esse nome.</p>
          : rows.map(row => {
              const on = chosen?.root === row.root && chosen.path === row.path;
              return <button key={`${row.root}|${row.path}`} role="option" aria-selected={on} className={row.depth ? 'iris-move-row' : 'iris-move-row iris-move-root'} title={row.full}
                style={{ paddingLeft: 10 + (filter ? 0 : row.depth * 14) }} onClick={() => setChosen({ root: row.root, path: row.path, label: row.name })}>
                {filter ? row.full : row.name}{!row.depth && <small>{row.athena ? 'Athena' : 'Sistema'}</small>}
              </button>;
            })}
      </div>
      {leavesVault && <p className="iris-move-warning">Saindo do cofre, os links das notas para estes arquivos deixam de encontrá-los.</p>}
      {items.some(i => i.file) && <p>Anexos do chat e imagens da Ayla ficam guardados na Ayla e não se movem.</p>}
      {error && <p role="alert" className="iris-upscale-error">{error}</p>}
      <div className="row-actions">
        <button className="primary-action" disabled={!chosen || busy} onClick={() => void move()}>{busy ? 'Movendo…' : chosen ? `Mover para “${chosen.label}”` : 'Escolha uma pasta'}</button>
        <button className="ghost-button" disabled={busy} onClick={onClose}>Cancelar</button>
      </div>
    </div>
  </div>;
}

import { useState } from 'react';
import { ImportIcon } from './Icons';
import { errorText, native } from '../services/Desktop';

/** Importar para a Athena, o mesmo diálogo nas Notas e na Biblioteca: arquivos escolhidos ou uma
 * pasta inteira (com as subpastas). Tudo é copiado; os originais ficam onde estão. */
export function ImportDialog({ books = false, root, folder, place, onDone, onClose }: {
  /** Biblioteca: `.epub`/`.cbz` para os Importados; senão, notas (`.txt`/`.md`), imagens, PDFs e vídeos na pasta. */
  books?: boolean; root?: string; folder?: string; place: string; onDone: () => void; onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState('');
  const [error, setError] = useState('');
  const [one, many] = books ? ['livro importado', 'livros importados'] : ['arquivo importado', 'arquivos importados'];
  async function pick(wholeFolder: boolean) {
    setBusy(true); setError(''); setResult('');
    try {
      const done = await native<{ imported: number; skipped: number; cancelled?: boolean }>(
        books ? 'library_import' : 'athena_import', books ? { wholeFolder } : { root, folder, wholeFolder });
      if (!done.cancelled) {
        const count = `${done.imported} ${done.imported === 1 ? one : many}`;
        setResult(done.skipped ? `${count}; ${done.skipped} ${done.skipped === 1 ? 'arquivo ignorado' : 'arquivos ignorados'}.` : `${count}.`);
        onDone();
      }
    } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  return <div className="dialog-backdrop" onClick={() => { if (!busy) onClose(); }}>
    <div className="dialog import-dialog" role="dialog" aria-modal="true" aria-labelledby="import-title" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape' && !busy) onClose(); }}>
      <h2 id="import-title"><ImportIcon size={18} /> {books ? 'Importar livros' : 'Importar para a Athena'}</h2>
      {books
        ? <p>Copia livros <b>.epub</b> e mangás <b>.cbz</b> para <b>{place}</b>. Os originais ficam onde estão, e nada é sobrescrito: nomes repetidos ganham um número.</p>
        : <p>Copia notas (<b>.md</b> e <b>.txt</b>, que viram .md), imagens, PDFs e vídeos para <b>{place}</b>. Os originais ficam onde estão, e nada é sobrescrito: nomes repetidos ganham um número.</p>}
      <div className="row-actions">
        <button type="button" className="primary-action" disabled={busy} onClick={() => void pick(false)}>Escolher arquivos…</button>
        <button type="button" disabled={busy} onClick={() => void pick(true)}>Escolher uma pasta…</button>
      </div>
      <p className="import-hint">{books ? 'Uma pasta entra inteira; cada subpasta vira uma série, com os capítulos em ordem.' : 'Uma pasta entra inteira, com as subpastas; só as notas, imagens, PDFs e vídeos dela são trazidos.'}</p>
      {busy && <p role="status">Importando…</p>}
      {result && <p role="status" className="import-result">{result}</p>}
      {error && <p role="alert" className="lock-error">{error}</p>}
      <div className="row-actions"><button type="button" className="ghost-button" disabled={busy} onClick={onClose}>{result ? 'Concluir' : 'Fechar'}</button></div>
    </div>
  </div>;
}

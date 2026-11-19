import { useEffect, useRef, useState } from 'react';
import { native, nativeTask, formatBytes, taskId, errorText, type CatalogEntry } from '../services/Desktop';

type Props = { entry: CatalogEntry; refresh: () => Promise<void>; action?: string };

/** Download de um modelo do catálogo: confirmação do tamanho, progresso e interrupção. */
export default function ModelSetup({ entry, refresh, action = 'Baixar' }: Props) {
  const [confirming, setConfirming] = useState(false); const [progress, setProgress] = useState<{ received: number; total: number } | null>(null);
  const [error, setError] = useState(''); const active = useRef<string | null>(null);
  useEffect(() => () => { if (active.current) void native('cancel_job', { requestId: active.current }); }, []);
  async function install() {
    setConfirming(false); setError(''); const requestId = taskId(); active.current = requestId;
    setProgress({ received: 0, total: entry.download_bytes });
    try {
      await nativeTask('install_catalog_model', { id: entry.id, requestId }, event => { if (event.type === 'download') setProgress({ received: event.received || 0, total: event.total || entry.download_bytes }); });
      await refresh();
    } catch (e) { setError(errorText(e)); }
    finally { active.current = null; setProgress(null); }
  }
  if (entry.installed) return <p className="setup-done">{entry.name} está pronto neste computador.</p>;
  return <div className="model-setup">
    {error && <p role="alert" className="app-error">{error}</p>}
    {progress ? <div role="status" className="setup-progress">
      <progress max={progress.total} value={progress.received} aria-label={`Baixando ${entry.name}`} />
      <span>{formatBytes(progress.received)} de {formatBytes(progress.total)}</span>
      <button onClick={() => { if (active.current) void native('cancel_job', { requestId: active.current }); }}>Interromper</button>
    </div> : confirming ? <div className="confirm-inline" role="group" aria-label="Confirmar download">
      <p>Baixar <b>{formatBytes(entry.download_bytes)}</b>? Os arquivos vêm das fontes oficiais (Hugging Face e, se faltar, o motor llama.cpp do GitHub) e têm a integridade verificada. Depois, a conversa funciona sem internet.</p>
      <button className="primary-action" onClick={() => void install()}>Confirmar download</button><button onClick={() => setConfirming(false)}>Cancelar</button>
    </div> : <button className="primary-action" disabled={!entry.fits} onClick={() => setConfirming(true)}>{action} · {formatBytes(entry.download_bytes)}</button>}
  </div>;
}

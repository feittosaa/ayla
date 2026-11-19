import { useEffect, useRef, useState } from 'react';
import { native, nativeTask, labels, formatBytes, taskId, errorText, capabilityList, type Capability, type Snapshot } from '../services/Desktop';
import ModelSetup from './ModelSetup';

type Props = { snapshot: Snapshot; refresh: () => Promise<void> };
export default function Models({ snapshot, refresh }: Props) {
  const [name, setName] = useState(''); const [capability, setCapability] = useState<Capability>('text');
  const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [progress, setProgress] = useState('');
  const [confirm, setConfirm] = useState<string | null>(null); const active = useRef<string | null>(null);
  useEffect(() => () => { if (active.current) void native('cancel_job', { requestId: active.current }); }, []);
  async function importModel() {
    if (busy) return; setBusy(true); setError(''); setProgress('Importando uma cópia do modelo…');
    try { await native('import_model', { name: name.trim() || labels[capability], capability }); await refresh(); }
    catch (e) { setError(errorText(e)); } finally { setBusy(false); setProgress(''); }
  }
  async function install(id: string) {
    setConfirm(null); setBusy(true); setError(''); const requestId = taskId(); active.current = requestId;
    try {
      await nativeTask('install_runtime', { id, requestId }, event => { if (event.type === 'download') setProgress(`${formatBytes(event.received || 0)} de ${formatBytes(event.total || 0)}`); });
      setProgress('Componente instalado e verificado.'); await refresh();
    } catch (e) { setError(errorText(e)); setProgress(''); }
    finally { active.current = null; setBusy(false); }
  }
  async function remove(id: string) {
    setConfirm(null); setBusy(true); setError('');
    try { await native('remove_model', { id }); await refresh(); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  const pending = snapshot.runtimes.find(r => r.id === confirm);
  const removed = snapshot.models.find(m => m.id === confirm);
  return <div className="dashboard">
    <p className="eyebrow">Seus modelos, suas escolhas</p><h1>Modelos locais</h1>
    <p>Importe qualquer modelo compatível. Nenhum é obrigatório. O arquivo original é preservado; a Ayla mantém uma cópia própria.</p>
    {error && <p role="alert" className="app-error">{error}</p>}
    {progress && <p role="status">{progress}</p>}
    {busy && active.current && <button onClick={() => void native('cancel_job', { requestId: active.current })}>Interromper download</button>}
    <section className="panel"><h2>Modelos recomendados</h2><p>Testados em português pela Ayla. O download só começa depois da sua confirmação.</p>
      {snapshot.catalog.map(entry => <article className="model-row" key={entry.id}><div><b>{entry.name}{entry.id === snapshot.recommended_model ? ' · recomendado' : ''}</b><small>{entry.summary}</small><small>Usa até {formatBytes(entry.memory_bytes)} de memória{entry.fits ? '' : ' · não cabe no orçamento atual'}</small></div><ModelSetup entry={entry} refresh={refresh} /></article>)}
    </section>
    <section className="panel"><h2>Importar modelo</h2>
      <label>Capacidade<select value={capability} disabled={busy} onChange={e => setCapability(e.target.value as Capability)}>{Object.entries(labels).map(([key, value]) => <option key={key} value={key}>{value}</option>)}</select></label>
      <label>Nome para identificar<input value={name} disabled={busy} onChange={e => setName(e.target.value)} placeholder="Nome do seu modelo" /></label>
      <p>{capability === 'speech' ? 'Escolha uma pasta de modelo VITS para sherpa-onnx, com arquivo ONNX, tokens.txt e dados auxiliares.' : capability === 'transcription' ? 'Escolha um modelo GGML do whisper.cpp.' : capability === 'vision' ? 'Escolha o modelo GGUF e, em seguida, seu projetor multimodal compatível.' : 'Escolha um arquivo GGUF compatível com llama.cpp.'}</p>
      <button className="primary-action" disabled={busy} onClick={() => void importModel()}>Escolher {capability === 'speech' ? 'pasta' : 'arquivo'} e importar</button>
    </section>
    <section className="panel"><h2>Biblioteca local</h2>
      {!snapshot.models.length && <p>Nenhum modelo importado. Você pode continuar usando a aplicação.</p>}
      {snapshot.models.map(model => <article className="model-row" key={model.id}><div><b>{model.name}</b><small>{capabilityList(model)} · {formatBytes(model.bytes)}</small><small>Compatibilidade de execução verificada ao carregar.</small></div><button disabled={busy} onClick={() => setConfirm(model.id)}>Remover cópia</button></article>)}
      <p>Orçamento estimado agora: {formatBytes(snapshot.memory_budget_bytes)}. O uso real depende da arquitetura, do contexto e dos buffers do modelo.</p>
      <button disabled={busy} onClick={() => { void native('unload_models').then(refresh).catch(e => setError(errorText(e))); }}>Liberar modelo carregado</button>
    </section>
    <section className="panel"><h2>Componentes de execução</h2><p>São os motores que executam os modelos. Baixe apenas os que quiser usar; nenhum modelo é incluído no download.</p>
      {snapshot.runtimes.map(runtime => <article className="model-row" key={runtime.id}><div><b>{runtime.label}</b><small>{runtime.installed ? 'Instalado' : formatBytes(runtime.bytes)}</small></div><button disabled={busy || runtime.installed} onClick={() => setConfirm(runtime.id)}>{runtime.installed ? 'Pronto' : 'Preparar'}</button></article>)}
    </section>
    {confirm && <div className="confirm-box" role="dialog" aria-modal="true" aria-label="Confirmar operação"><h2>{pending ? 'Preparar componente' : 'Remover cópia local'}</h2><p>{pending ? `${pending.label}: download de ${formatBytes(pending.bytes)}, a partir da distribuição oficial, com verificação de integridade.` : `Remover a cópia de “${removed?.name}” mantida pela Ayla? O arquivo original não será alterado.`}</p><button onClick={() => { if (pending) void install(pending.id); else if (removed) void remove(removed.id); }}>Confirmar</button><button onClick={() => setConfirm(null)}>Cancelar</button></div>}
  </div>;
}

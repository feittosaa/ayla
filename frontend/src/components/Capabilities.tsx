import { useEffect, useRef, useState } from 'react';
import { native, taskId, errorText, type Sensitivity, type Snapshot, type CapabilityRequest } from '../services/Desktop';
import { recordWav } from '../services/Audio';

export default function Capabilities({ snapshot, refresh }: { snapshot: Snapshot; refresh: () => Promise<void> }) {
  const [tool, setTool] = useState<'voice' | 'memory'>('voice'); const [text, setText] = useState(''); const [output, setOutput] = useState('');
  const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  const [cloud, setCloud] = useState(false); const [reviewed, setReviewed] = useState(false); const [sensitivity, setSensitivity] = useState<Sensitivity>('uncertain');
  const [cloudModel, setCloudModel] = useState(''); const [recording, setRecording] = useState(false);
  const mounted = useRef(true);
  const transcriptionReady = snapshot.models.some(m => m.capability === 'transcription') && snapshot.runtimes.some(r => r.id === 'whisper' && r.installed);
  const active = useRef<string | null>(null); const recorder = useRef<{ stop: () => void } | null>(null);
  const useCloud = cloud || snapshot.preferences.usage_mode === 'cloud_only';
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; recorder.current?.stop(); if (active.current) void native('cancel_job', { requestId: active.current }); }; }, []);
  async function run(command: 'voice_task' | 'semantic_memory', extra: Record<string, unknown>, encoded?: string) {
    if (active.current) return;
    const id = taskId(); active.current = id; setBusy(true); setError(''); setOutput('');
    const request: CapabilityRequest = { id, text, sensitivity, reviewed, request_cloud: cloud, cloud_model: cloudModel || null };
    try {
      const result = await native<{ text?: string; saved?: boolean; results?: { content: string; score: number }[] }>(command, command === 'voice_task' ? { request, audio: encoded || null } : { request, ...extra });
      if (result.text) { setOutput(result.text); setText(result.text); setReviewed(false); }
      if (result.saved) setOutput('Memória salva neste computador.');
      if (result.results) setOutput(result.results.length ? result.results.map(item => item.content).join('\n\n') : 'Nenhuma memória compatível encontrada.');
    } catch (e) { setError(errorText(e)); } finally { active.current = null; setBusy(false); setReviewed(false); await refresh(); }
  }
  async function record() {
    if (recording) { recorder.current?.stop(); return; }
    try { setRecording(true); const session = await recordWav(); if (!mounted.current) { session.stop(); return; } recorder.current = session; const audio = await session.result; if (!mounted.current) return; setRecording(false); recorder.current = null; await run('voice_task', {}, audio); }
    catch (e) { setRecording(false); recorder.current = null; setError(errorText(e)); }
  }
  return <div className="dashboard"><p className="eyebrow">Capacidades sempre acessíveis</p><h1>Transcrição e memória</h1>
    <div className="button-row"><button aria-pressed={tool === 'voice'} disabled={busy || recording} onClick={() => setTool('voice')}>Voz</button><button aria-pressed={tool === 'memory'} disabled={busy || recording} onClick={() => setTool('memory')}>Memória semântica</button></div>
    <p>{tool === 'voice' ? 'Transcreva áudio neste computador. O áudio nunca vai para a nuvem.' : 'Salve notas e encontre conteúdo por significado. Memórias ficam neste computador; a busca usa o mesmo modelo da gravação.'}</p>
    {tool === 'voice' && !transcriptionReady && <p>Para gravar e transcrever, prepare o componente de transcrição e importe um modelo em Modelos.</p>}
    {error && <p role="alert" className="app-error">{error}</p>}
    <section className="panel">
      <label>{tool === 'voice' ? 'Transcrição' : 'Nota ou busca'}<textarea value={text} disabled={busy || recording} onChange={e => { setText(e.target.value); setReviewed(false); }} rows={6} /></label>
      <label>Privacidade<select value={sensitivity} disabled={busy || recording} onChange={e => { setSensitivity(e.target.value as Sensitivity); setReviewed(false); }}><option value="uncertain">Ainda não verificada</option><option value="sensitive">Sensível · somente local</option><option value="public">Público</option></select></label>
      <label><input type="checkbox" checked={useCloud} disabled={busy || recording || ['local_only', 'cloud_only'].includes(snapshot.preferences.usage_mode)} onChange={e => { setCloud(e.target.checked); setReviewed(false); }} />Solicitar provedor cloud</label>
      {useCloud && <div className="cloud-review"><p>Será enviado somente o texto mostrado acima. Áudio bruto e notas já salvas permanecem locais.</p><label>Modelo deste recurso no provedor<input value={cloudModel} onChange={e => setCloudModel(e.target.value)} /></label><label><input type="checkbox" checked={reviewed} disabled={sensitivity !== 'public'} onChange={e => setReviewed(e.target.checked)} />Revisei o texto e confirmo que é público.</label></div>}
      <div className="button-row">
        {tool === 'voice' ? <><button disabled={busy || recording || useCloud} onClick={() => void run('voice_task', {})}>Transcrever arquivo WAV</button><button disabled={busy || useCloud || !transcriptionReady} onClick={() => void record()}>{recording ? 'Parar gravação' : 'Gravar até trinta segundos'}</button></> : <><button disabled={busy || !text || (useCloud && !reviewed)} onClick={() => void run('semantic_memory', { search: false })}>Guardar nota</button><button disabled={busy || !text || (useCloud && !reviewed)} onClick={() => void run('semantic_memory', { search: true })}>Buscar por significado</button></>}
        {busy && <button onClick={() => { if (active.current) void native('cancel_job', { requestId: active.current }); }}>Interromper</button>}
      </div>
      {busy && <p role="status">Processando…</p>}{output && <pre className="task-output" aria-live="polite">{output}</pre>}
    </section>
  </div>;
}

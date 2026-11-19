// Gravação iniciada somente por clique; limita duração e libera o microfone.
export async function recordWav(): Promise<{ stop: () => void; result: Promise<string> }> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  let recorder: MediaRecorder;
  try { recorder = new MediaRecorder(stream); }
  catch { stream.getTracks().forEach(track => track.stop()); throw new Error('A gravação de áudio não está disponível neste ambiente.'); }
  const pieces: BlobPart[] = [];
  let timer: ReturnType<typeof setTimeout>;
  const result = new Promise<string>((resolve, reject) => {
    recorder.ondataavailable = event => { if (event.data.size) pieces.push(event.data); };
    recorder.onerror = () => { clearTimeout(timer); stream.getTracks().forEach(t => t.stop()); reject(new Error('Não foi possível gravar o áudio.')); };
    recorder.onstop = () => {
      clearTimeout(timer); stream.getTracks().forEach(t => t.stop());
      void (async () => {
        const context = new AudioContext();
        try {
          const decoded = await context.decodeAudioData(await new Blob(pieces, { type: recorder.mimeType }).arrayBuffer());
          const frames = Math.ceil(decoded.duration * 16000);
          const offline = new OfflineAudioContext(1, frames, 16000);
          const source = offline.createBufferSource(); source.buffer = decoded; source.connect(offline.destination); source.start();
          const buffer = (await offline.startRendering()).getChannelData(0);
          const wav = new ArrayBuffer(44 + buffer.length * 2); const view = new DataView(wav);
          function text(at: number, value: string) { for (let i = 0; i < value.length; i++) view.setUint8(at + i, value.charCodeAt(i)); }
          text(0, 'RIFF'); view.setUint32(4, 36 + buffer.length * 2, true); text(8, 'WAVE'); text(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true); view.setUint32(24, 16000, true); view.setUint32(28, 32000, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true); text(36, 'data'); view.setUint32(40, buffer.length * 2, true);
          for (let i = 0; i < buffer.length; i++) view.setInt16(44 + i * 2, Math.max(-1, Math.min(1, buffer[i])) * 32767, true);
          let binary = ''; const bytes = new Uint8Array(wav);
          for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
          resolve(btoa(binary));
        } finally { await context.close(); }
      })().catch(reject);
    };
  });
  recorder.start(); timer = setTimeout(() => { if (recorder.state !== 'inactive') recorder.stop(); }, 30000);
  return { stop: () => { if (recorder.state !== 'inactive') recorder.stop(); }, result };
}

// Gravação iniciada somente por clique; limita duração e libera o microfone.
// Captura PCM pela Web Audio API (sem MediaRecorder/codecs, que falham no WebKitGTK)
// e entrega WAV mono 16 kHz em base64, pronto para o whisper.cpp.

/** WAV PCM16 mono a partir de amostras float [-1, 1]. */
export function encodeWav(samples: Float32Array, rate: number): Uint8Array {
  const wav = new ArrayBuffer(44 + samples.length * 2); const view = new DataView(wav);
  const text = (at: number, value: string) => { for (let i = 0; i < value.length; i++) view.setUint8(at + i, value.charCodeAt(i)); };
  text(0, 'RIFF'); view.setUint32(4, 36 + samples.length * 2, true); text(8, 'WAVE'); text(12, 'fmt ');
  view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true); view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true); text(36, 'data'); view.setUint32(40, samples.length * 2, true);
  for (let i = 0; i < samples.length; i++) view.setInt16(44 + i * 2, Math.round(Math.max(-1, Math.min(1, samples[i])) * 32767), true);
  return new Uint8Array(wav);
}

/** Reamostragem linear simples para 16 kHz (fala; sem dependência de OfflineAudioContext). */
export function resample(input: Float32Array, from: number, to = 16000): Float32Array {
  if (from === to) return input;
  const output = new Float32Array(Math.floor(input.length * to / from));
  const ratio = from / to;
  for (let i = 0; i < output.length; i++) {
    const position = i * ratio; const index = Math.floor(position); const fraction = position - index;
    output[i] = input[index] * (1 - fraction) + (input[index + 1] ?? input[index]) * fraction;
  }
  return output;
}

export function base64(bytes: Uint8Array) {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return btoa(binary);
}

export type Recording = { stop: () => void; result: Promise<string>; peek: () => string | null; reachedLimit: () => boolean };

/** Uma fala pode ter até 5 minutos (cabe na prévia ao vivo e transcreve em poucos segundos). */
export const RECORD_LIMIT_MS = 10 * 60 * 1000;

export async function recordWav(limitMs = RECORD_LIMIT_MS): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
  const context = new AudioContext();
  await context.resume();
  const source = context.createMediaStreamSource(stream);
  const processor = context.createScriptProcessor(4096, 1, 1);
  const chunks: Float32Array[] = []; let length = 0;
  processor.onaudioprocess = event => { const data = event.inputBuffer.getChannelData(0); chunks.push(new Float32Array(data)); length += data.length; };
  source.connect(processor); processor.connect(context.destination);
  let finished = false; let limited = false;
  const collect = () => { const samples = new Float32Array(length); let offset = 0; for (const chunk of chunks) { samples.set(chunk, offset); offset += chunk.length; } return samples; };
  let finish: () => void = () => {};
  const result = new Promise<string>((resolve, reject) => {
    finish = () => {
      if (finished) return; finished = true; clearTimeout(timer);
      processor.onaudioprocess = null; source.disconnect(); processor.disconnect();
      stream.getTracks().forEach(track => track.stop());
      const rate = context.sampleRate; void context.close();
      if (length < rate * 0.3) { reject(new Error('Gravação curta demais. Toque no microfone, fale e toque de novo.')); return; }
      resolve(base64(encodeWav(resample(collect(), rate), 16000)));
    };
  });
  const timer = setTimeout(() => { limited = true; finish(); }, limitMs);
  // Áudio gravado até agora, para a prévia ao vivo.
  const peek = () => finished || length < context.sampleRate * 0.5 ? null : base64(encodeWav(resample(collect(), context.sampleRate), 16000));
  return { stop: () => finish(), result, peek, reachedLimit: () => limited };
}

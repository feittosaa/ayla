import { native, taskId } from './Desktop';
import { splitSentences } from './Sentences';

/**
 * Fala respostas frase a frase: sintetiza a próxima enquanto a atual toca.
 * `stop` interrompe a fila, a síntese em andamento e o áudio.
 */
export class Speaker {
  private buffer = '';
  private generation = 0;
  private synthesis: Promise<unknown> = Promise.resolve();
  private playback: Promise<void> = Promise.resolve();
  private audio: HTMLAudioElement | null = null;
  private active = new Set<string>();
  private pending = 0;
  constructor(private readonly onChange: (speaking: boolean) => void, private readonly onError: (message: string) => void) {}

  /** Texto parcial de uma resposta em geração. */
  push(chunk: string) {
    this.buffer += chunk;
    const { sentences, rest } = splitSentences(this.buffer);
    this.buffer = rest;
    sentences.forEach(sentence => this.enqueue(sentence));
  }
  /** Fim da resposta: fala o que restou. */
  flush() {
    const rest = this.buffer.trim(); this.buffer = '';
    if (rest.length > 1) this.enqueue(rest);
  }
  /** Fala um texto completo (botão "ouvir"). */
  speakAll(text: string) { this.stop(); this.push(text); this.flush(); }

  stop() {
    this.generation += 1; this.buffer = ''; this.pending = 0;
    this.active.forEach(id => void native('cancel_job', { requestId: id }));
    this.active.clear();
    if (this.audio) { this.audio.pause(); this.audio = null; }
    this.synthesis = Promise.resolve(); this.playback = Promise.resolve();
    this.onChange(false);
  }

  private enqueue(sentence: string) {
    const generation = this.generation;
    this.pending += 1; this.onChange(true);
    const audio = this.synthesis.then(async () => {
      if (generation !== this.generation) return null;
      const id = taskId(); this.active.add(id);
      try { return (await native<{ audio: string | null }>('speak', { requestId: id, text: sentence })).audio; }
      finally { this.active.delete(id); }
    });
    this.synthesis = audio.catch(() => null);
    this.playback = this.playback.then(async () => {
      try {
        const encoded = await audio;
        if (encoded && generation === this.generation) await this.play(encoded, generation);
      } catch (e) {
        if (generation === this.generation) { this.onError(e instanceof Error ? e.message : String(e)); this.stop(); }
      } finally {
        if (generation === this.generation) { this.pending -= 1; if (this.pending <= 0) this.onChange(false); }
      }
    });
  }
  private play(encoded: string, generation: number) {
    return new Promise<void>(resolve => {
      const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
      const url = URL.createObjectURL(new Blob([bytes], { type: 'audio/wav' }));
      const audio = new Audio(url); this.audio = audio;
      const done = () => { URL.revokeObjectURL(url); if (this.audio === audio) this.audio = null; resolve(); };
      audio.onended = done; audio.onerror = done; audio.onpause = () => { if (generation !== this.generation) done(); };
      void audio.play().catch(done);
    });
  }
}

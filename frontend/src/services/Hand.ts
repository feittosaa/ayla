// Tipos e textos do Hand, usados pela página e pelos cartões da conversa.
/** Passo de um plano do Hand (igual ao núcleo). */
export type HandStep = { acao: 'pasta'; caminho: string } | { acao: 'mover'; de: string; para: string } | { acao: 'lixeira'; caminho: string }
  | { acao: 'iris'; arquivo: string; nome: string; para: string };
export type Lote = { id: string; quando: number; titulo: string; feitos: HandStep[]; desfeito: boolean; origem?: string | null };
export type Resultado = { lote: Lote; falhas: [HandStep, string][] };
export type Achado = { path: string; name: string; folder: boolean; bytes: number; modified: number };
export type Saida = { code: number | null; stdout: string; stderr: string; cortado: boolean; tempo_esgotado: boolean };

export const nameOf = (path: string) => path.slice(path.lastIndexOf('/') + 1);
/** Caminho curto: "~/Downloads/x.pdf". */
export function short(path: string, home: string) { return home && path.startsWith(`${home}/`) ? `~${path.slice(home.length)}` : path; }
export function size(bytes: number) { return bytes < 1024 ? `${bytes} B` : bytes < 1 << 20 ? `${(bytes / 1024).toFixed(0)} KB` : bytes < 1 << 30 ? `${(bytes / (1 << 20)).toFixed(1)} MB` : `${(bytes / (1 << 30)).toFixed(1)} GB`; }
export function when(seconds: number) { return new Date(seconds * 1000).toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' }); }

/** Pasta comum dos passos (a que está sendo arrumada), para mostrar caminhos curtos. */
export function baseOf(plan: HandStep[]) {
  const first = plan.find(s => s.acao === 'mover');
  if (!first || first.acao !== 'mover') return undefined;
  const dir = first.de.slice(0, first.de.lastIndexOf('/'));
  return plan.every(s => (s.acao === 'mover' ? [s.de, s.para] : s.acao === 'iris' ? [s.para] : [s.caminho]).every(p => p.startsWith(`${dir}/`))) ? dir : undefined;
}

import { Channel, invoke, isTauri } from '@tauri-apps/api/core';

export type Profile = 'light' | 'moderate' | 'maximum';
export type Capability = 'text' | 'reasoning' | 'vision' | 'transcription' | 'speech' | 'embedding';
export type UsageMode = 'local_only' | 'cloud_only' | 'hybrid' | 'per_conversation' | 'explicit_request' | 'suggest';
export type Sensitivity = 'public' | 'sensitive' | 'uncertain';
export type Preferences = {
  personality: string; version: number; onboarding_complete: boolean; profile: Profile | null;
  resource_percent: number | null; usage_mode: UsageMode; cloud_provider: string; cloud_endpoint: string; cloud_model: string; cloud_asked: boolean; image_provider: string; image_model: string;
  history_messages: number; history_chars: number; context_tokens: number; context_auto?: boolean | null; idle_seconds: number; memory_auto: boolean; voice: boolean | null;
  desktop: { autostart: boolean; background: boolean; system_theme: boolean; panel_color: string; panel_accent: string; panel_font: string; panel_radius: number };
  notifications: { reply: boolean; setup: boolean; reminders: boolean };
  athena_folder: string; athena_vaults: string[]; athena_git: boolean;
  response_source: '' | 'local' | 'auto' | 'cloud';
  hidden_sources?: string[];
  appearance: { mode: 'escuro' | 'claro' | 'sistema'; accent: string; dark_background: string; light_background: string; roundness: number; opacity: number; icon_ayla: number; icon_athena: number };
};
export type Model = { id: string; name: string; capability: Capability; capabilities?: Capability[]; companion: string | null; bytes: number; sha256: string; catalog?: string | null };
export type CatalogEntry = { id: string; name: string; summary: string; capabilities: Capability[]; download_bytes: number; memory_bytes: number; installed: boolean; fits: boolean; gpu_bytes?: number; gpu_only?: boolean };
export type Hardware = {
  ram_total_bytes: number; ram_available_bytes: number; logical_cpus: number;
  gpus: { vendor: string; vram_total_bytes: number | null; vram_available_bytes: number | null }[];
};
export type Metric = { model: string; first_token_ms: number | null; duration_ms: number; chunks: number; cancelled: boolean; prompt_tokens?: number | null; cache_tokens?: number | null; prompt_tps?: number | null; decode_tps?: number | null };
export type Snapshot = {
  system_prompt: string; preferences: Preferences; hardware: Hardware; models: Model[]; memory_budget_bytes: number; gpu_budget_bytes?: number;
  runtimes: { id: string; label: string; bytes: number; installed: boolean }[];
  worker: { active_model: string | null }; metrics: Metric[]; recommended_profile: Profile;
  catalog: CatalogEntry[]; recommended_model: string | null;
  setup: SetupState | null;
  projects: Project[];
};
export type SetupState = { active: boolean; label?: string; received?: number; total?: number; error?: string; cancelled?: boolean };
export type CloudProvider = { id: string; name: string; summary: string; login: 'key' | 'open_router' | 'chat_gpt'; keys_url: string | null; default_model: string | null; images: 'generations' | 'open_router_chat' | null; connected: boolean; account: string | null };
export type Privacy = 'sensitive' | 'uncertain' | 'public';
export type Fact = { id: string; content: string; kind: string; privacy: Privacy; origin: string; conversation_id: string | null; created_at: number; updated_at: number; project_id: string | null };
export type Project = { id: string; name: string; description: string; created_at: number; updated_at: number; conversations: number; memories: number };
export type MemoryList = { facts: Fact[]; enabled: boolean; ready: boolean };
export type Conversation = { id: string; title: string; cloud_consent: boolean; project_id: string | null };
export type FileKind = 'image' | 'gif' | 'pdf' | 'text' | 'audio' | 'video';
export type FileInfo = { id: string; name: string; kind: FileKind; bytes: number; created_at: number; conversation_id: string | null; project_id: string | null; message_id: number | null };
export const fileLabels: Record<FileKind, string> = { image: 'Imagem', gif: 'GIF', pdf: 'PDF', text: 'Texto', audio: 'Áudio', video: 'Vídeo' };
export type ReplyOrigin = 'cloud' | 'cloud_search';
export type Message = { origin?: ReplyOrigin | null; role: 'user' | 'assistant'; content: string; complete: boolean; attachments?: FileInfo[]; sources?: Source[]; images?: ImageResults; notice?: string; extras?: { images?: ImageResults; sources?: Source[] } | null };
/** Imagem encontrada numa busca: fonte, autor e licença ("livre" ou só "referencia"). */
export type ImageItem = { provider: string; id: string; thumb: string; full: string; page: string; author: string; author_url: string; license: 'livre' | 'referencia'; license_name: string; license_url: string; width: number; height: number; download_ping?: string };
export type ImageResults = { query: string; items: ImageItem[]; links: { name: string; url: string }[] };
/** Arquivo usado pela Iris numa resposta, com as páginas dos trechos. */
export type Source = { id: string; name: string; pages: number[]; date?: string; url?: string; note?: string };
export type TaskEvent = { type: 'token' | 'download' | 'suggestion' | 'routing' | 'memory' | 'memory_error' | 'status' | 'sources' | 'origin' | 'images' | 'progress'; origin?: ReplyOrigin; percent?: number; text?: string; received?: number; total?: number; facts?: Fact[]; files?: Source[] } & Partial<ImageResults>;
export type CapabilityRequest = { id: string; text: string; sensitivity: Sensitivity; reviewed: boolean; request_cloud: boolean; cloud_model: string | null };
/** Esforço de raciocínio: na nuvem vai para a plataforma; neste computador escolhe entre resposta direta e raciocínio. */
export type Effort = '' | 'low' | 'medium' | 'high';
/** Memória pública que pode acompanhar um envio à nuvem, se continuar marcada na revisão. */
export type Reference = { id: string; content: string };
export type ChatRequest = Omit<CapabilityRequest, 'text'> & { conversation: string; message: string; personality_preview: string | null; web_search: boolean; automatic: boolean; auto_route?: 'local' | 'cloud' | 'search' | null; capability: Capability | null; model: string | null; attachments: string[]; effort: Effort | null; references: Reference[]; bases?: string[] };
export const labels: Record<Capability, string> = { text: 'Conversa', reasoning: 'Raciocínio', vision: 'Visão', transcription: 'Transcrição', speech: 'Voz de saída (sem uso)', embedding: 'Memória semântica' };
export const modeLabels: Record<UsageMode, string> = { local_only: 'Somente local', cloud_only: 'Somente nuvem', hybrid: 'Local + nuvem', per_conversation: 'Autorizar por conversa', explicit_request: 'Nuvem quando solicitada', suggest: 'Ayla sugere; eu decido' };

export async function desktopSnapshot(): Promise<Snapshot> {
  if (!isTauri()) throw new Error('Abra a Ayla pelo aplicativo desktop.');
  return invoke('desktop_snapshot');
}
export async function savePreferences(preferences: Preferences) { await invoke('save_preferences', { preferences }); }
export function native<T>(command: string, args?: Record<string, unknown>) { return invoke<T>(command, args); }
export async function nativeTask<T>(command: string, args: Record<string, unknown>, onEvent: (event: TaskEvent) => void) {
  const events = new Channel<TaskEvent>(); events.onmessage = onEvent;
  return invoke<T>(command, { ...args, events });
}
export function supports(model: Model, kind: Capability) {
  return (model.capability === kind || Boolean(model.capabilities?.includes(kind))) && (kind !== 'vision' || Boolean(model.companion));
}
export function capabilityList(model: Model) {
  return [...new Set([model.capability, ...(model.capabilities || [])])].filter(kind => supports(model, kind)).map(kind => labels[kind]).join(', ');
}
export function taskId() { return crypto.randomUUID(); }
export function gib(bytes: number) { return (bytes / 2 ** 30).toFixed(1); }
export function formatBytes(bytes: number) { return bytes > 1024 ** 3 ? `${gib(bytes)} GB` : `${(bytes / 1024 ** 2).toFixed(1)} MB`; }
export function errorText(error: unknown) { return error instanceof Error ? error.message : String(error); }

import { Channel, invoke, isTauri } from '@tauri-apps/api/core';

export type Profile = 'light' | 'moderate' | 'maximum';
export type Capability = 'text' | 'reasoning' | 'vision' | 'transcription' | 'speech' | 'embedding';
export type UsageMode = 'local_only' | 'cloud_only' | 'hybrid' | 'per_conversation' | 'explicit_request' | 'suggest';
export type Sensitivity = 'public' | 'sensitive' | 'uncertain';
export type Preferences = {
  personality: string; version: number; onboarding_complete: boolean; profile: Profile | null;
  resource_percent: number | null; usage_mode: UsageMode; cloud_endpoint: string; cloud_model: string;
  history_messages: number; history_chars: number; context_tokens: number; idle_seconds: number; memory_auto: boolean; voice: boolean | null;
};
export type Model = { id: string; name: string; capability: Capability; capabilities?: Capability[]; companion: string | null; bytes: number; sha256: string; catalog?: string | null };
export type CatalogEntry = { id: string; name: string; summary: string; capabilities: Capability[]; download_bytes: number; memory_bytes: number; installed: boolean; fits: boolean };
export type Hardware = {
  ram_total_bytes: number; ram_available_bytes: number; logical_cpus: number;
  gpus: { vendor: string; vram_total_bytes: number | null; vram_available_bytes: number | null }[];
};
export type Metric = { model: string; first_token_ms: number | null; duration_ms: number; chunks: number; cancelled: boolean };
export type Snapshot = {
  system_prompt: string; preferences: Preferences; hardware: Hardware; models: Model[]; memory_budget_bytes: number;
  runtimes: { id: string; label: string; bytes: number; installed: boolean }[];
  worker: { active_model: string | null }; metrics: Metric[]; recommended_profile: Profile;
  catalog: CatalogEntry[]; recommended_model: string | null;
  setup: SetupState | null;
};
export type SetupState = { active: boolean; label?: string; received?: number; total?: number; error?: string; cancelled?: boolean };
export type Privacy = 'sensitive' | 'uncertain' | 'public';
export type Fact = { id: string; content: string; kind: string; privacy: Privacy; origin: string; conversation_id: string | null; created_at: number; updated_at: number };
export type MemoryList = { facts: Fact[]; enabled: boolean; ready: boolean };
export type Conversation = { id: string; title: string; cloud_consent: boolean };
export type Message = { role: 'user' | 'assistant'; content: string; complete: boolean };
export type TaskEvent = { type: 'token' | 'download' | 'suggestion' | 'routing' | 'memory' | 'memory_error'; text?: string; received?: number; total?: number; facts?: Fact[] };
export type CapabilityRequest = { id: string; text: string; sensitivity: Sensitivity; reviewed: boolean; request_cloud: boolean; cloud_model: string | null };
export type ChatRequest = Omit<CapabilityRequest, 'text' | 'cloud_model'> & { conversation: string; message: string; personality_preview: string | null; capability: Capability | null; model: string | null; image: string | null };
export const labels: Record<Capability, string> = { text: 'Conversa', reasoning: 'Raciocínio', vision: 'Visão', transcription: 'Transcrição', speech: 'Síntese de voz', embedding: 'Memória semântica' };
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

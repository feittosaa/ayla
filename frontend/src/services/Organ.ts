import { useCallback, useEffect, useState } from 'react';
import { native } from './Desktop';

/** Estado de um órgão (produto separado que a pessoa escolheu baixar), vindo do núcleo. */
export type OrganStatus = {
  id: string; name: string; description: string; version: string;
  installed: boolean; task: string | null; error: string | null; engine: boolean;
  running: boolean; container: boolean; locked: boolean; contract_ok: boolean;
  standalone: boolean; downloaded: boolean; icon: string | null;
};

/** Página da Moneta servida pela camada de compatibilidade da Ayla (protocolo próprio do núcleo). */
export const monetaPage = navigator.userAgent.includes('Windows') ? 'http://moneta.localhost/' : 'moneta://localhost/';

/** Aparência atual da Ayla repassada à página da Moneta: o núcleo a serve na folha de
 * compatibilidade (primeira pintura) e a página aberta recebe as mudanças por mensagem. */
type Look = { vars: Record<string, string>; light: boolean };
let look: Look | null = null;
export function shareAppearance(vars: Record<string, string>, light: boolean) {
  const next = { vars: Object.fromEntries(Object.entries(vars).filter(([name]) => name.startsWith('--ayla-'))), light };
  if (look && look.light === next.light && JSON.stringify(look.vars) === JSON.stringify(next.vars)) return;
  look = next;
  void native('organ_theme', look).catch(() => { /* sem núcleo: a Moneta fica com o tema padrão */ });
  window.dispatchEvent(new Event('ayla-tema'));
}
/** Envia a aparência atual para a página da Moneta (só cores; o destino é a página embutida). */
export function postAppearance(frame: HTMLIFrameElement | null) {
  if (look) frame?.contentWindow?.postMessage({ type: 'ayla-tema', ...look }, '*');
}

export function useOrgan() {
  const [status, setStatus] = useState<OrganStatus | null>(null);
  const refresh = useCallback(async () => {
    try { setStatus(await native<OrganStatus>('organ_status')); } catch { /* sem núcleo: o órgão só não aparece */ }
  }, []);
  useEffect(() => {
    let alive = true;
    native<OrganStatus>('organ_status').then(next => { if (alive) setStatus(next); }, () => { /* sem núcleo: o órgão só não aparece */ });
    return () => { alive = false; };
  }, []);
  // Acompanha downloads e inícios enquanto houver uma etapa em andamento.
  useEffect(() => {
    if (!status?.task) return;
    const timer = setInterval(() => void refresh(), 1500);
    return () => clearInterval(timer);
  }, [status?.task, refresh]);
  return { status, refresh };
}

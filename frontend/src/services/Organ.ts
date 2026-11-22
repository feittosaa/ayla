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

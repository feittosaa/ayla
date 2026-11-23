/** Calendário da Athena: avisos, hora (24 h) e data (DD/MM/AAAA), sem depender do idioma do sistema. */

export type AlertUnit = 'na-hora' | 'minutos' | 'horas' | 'dias';
export type Alert = { unit: AlertUnit; amount: number };

const PER: Record<Exclude<AlertUnit, 'na-hora'>, number> = { minutos: 1, horas: 60, dias: 1440 };
/** Maior antecedência aceita pelo núcleo: 365 dias. */
export const MAX_ALERT_MINUTES = 365 * 1440;

export function alertMinutes(alert: Alert): number | null {
  if (alert.unit === 'na-hora') return 0;
  if (!Number.isInteger(alert.amount) || alert.amount < 1) return null;
  const minutes = alert.amount * PER[alert.unit];
  return minutes <= MAX_ALERT_MINUTES ? minutes : null;
}

/** Minutos → a unidade mais natural (90 → 90 minutos; 120 → 2 horas; 2880 → 2 dias). */
export function alertFrom(minutes: number): Alert {
  if (minutes === 0) return { unit: 'na-hora', amount: 1 };
  if (minutes % 1440 === 0) return { unit: 'dias', amount: minutes / 1440 };
  if (minutes % 60 === 0) return { unit: 'horas', amount: minutes / 60 };
  return { unit: 'minutos', amount: minutes };
}

/** "1 hora antes e na hora", na ordem em que os avisos chegam. */
export function alertsLabel(minutes: number[]): string {
  if (!minutes.length) return '';
  const one = (m: number) => {
    const { unit, amount } = alertFrom(m);
    if (unit === 'na-hora') return 'na hora';
    const name = unit === 'dias' ? (amount === 1 ? 'dia' : 'dias') : unit === 'horas' ? (amount === 1 ? 'hora' : 'horas') : (amount === 1 ? 'minuto' : 'minutos');
    return `${amount} ${name} antes`;
  };
  const list = [...minutes].sort((a, b) => b - a).map(one);
  return list.length === 1 ? list[0] : `${list.slice(0, -1).join(', ')} e ${list[list.length - 1]}`;
}

/** Hora digitada → "HH:MM" (24 h), ou `null`. Aceita "9", "930", "9:30", "09h30", "21:05". */
export function parseTime(text: string): string | null {
  const digits = text.trim().replace(/h$/i, '').replace(/[h.]/i, ':');
  const match = /^(\d{1,2})(?::?(\d{2}))?$/.exec(digits);
  if (!match) return null;
  const [h, m] = [Number(match[1]), Number(match[2] ?? 0)];
  return h <= 23 && m <= 59 ? `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}` : null;
}

/** Máscara enquanto digita: "1530" → "15:30". */
export function maskTime(text: string): string {
  const digits = text.replace(/\D/g, '').slice(0, 4);
  return digits.length > 2 ? `${digits.slice(0, 2)}:${digits.slice(2)}` : digits;
}

/** "AAAA-MM-DD" → "DD/MM/AAAA". */
export const showDate = (iso: string) => { const [y, m, d] = iso.split('-'); return `${d}/${m}/${y}`; };

/** "DD/MM/AAAA" (ou "D/M/AAAA") → "AAAA-MM-DD", só se a data existir. */
export function parseDate(text: string): string | null {
  const match = /^(\d{1,2})\/(\d{1,2})\/(\d{4})$/.exec(text.trim());
  if (!match) return null;
  const [d, m, y] = [Number(match[1]), Number(match[2]), Number(match[3])];
  const date = new Date(y, m - 1, d);
  if (date.getFullYear() !== y || date.getMonth() !== m - 1 || date.getDate() !== d) return null;
  return `${y}-${String(m).padStart(2, '0')}-${String(d).padStart(2, '0')}`;
}

/** Máscara enquanto digita: "23112026" → "23/11/2026". */
export function maskDate(text: string): string {
  const digits = text.replace(/\D/g, '').slice(0, 8);
  return [digits.slice(0, 2), digits.slice(2, 4), digits.slice(4)].filter(Boolean).join('/');
}

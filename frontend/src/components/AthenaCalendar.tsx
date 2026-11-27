import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent, type InputHTMLAttributes } from 'react';
import { listen } from '@tauri-apps/api/event';
import Confirm from './Confirm';
import { ChevronLeftIcon, ChevronRightIcon, CloseIcon, FileIcon, PencilIcon, PlusIcon } from './Icons';
import { errorText, native } from '../services/Desktop';
import { alertFrom, alertMinutes, alertsLabel, maskDate, maskTime, maskedEdit, parseDate, parseTime, showDate, type Alert, type AlertUnit } from '../services/Calendar';
import './AthenaCalendar.css';

/** Uma ocorrência num dia (os itens que repetem aparecem em cada dia). */
type Item = { id: string; text: string; date: string; time: string | null; repeat: string; notify: boolean; past: boolean; start_date: string; repeat_label: string; alerts: number[] };
type Range = { items: Item[]; notes: string[] };
/** Data e hora como digitadas (DD/MM/AAAA e HH:MM, 24 h); convertidas ao salvar. */
type Draft = { id: string | null; text: string; date: string; allDay: boolean; time: string; repeat: string; alerts: Alert[] };
const UNITS: [AlertUnit, string][] = [['na-hora', 'Na hora do evento'], ['minutos', 'Minutos antes'], ['horas', 'Horas antes'], ['dias', 'Dias antes']];

const WEEK = ['seg', 'ter', 'qua', 'qui', 'sex', 'sáb', 'dom'];
const REPEATS: [string, string][] = [['none', 'Não repete'], ['daily', 'Todo dia'], ['weekdays', 'Dias úteis (seg–sex)'], ['weekly', 'Toda semana'], ['monthly', 'Todo mês'], ['yearly', 'Todo ano']];
const pad2 = (n: number) => String(n).padStart(2, '0');
const iso = (d: Date) => `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
const parse = (text: string) => { const [y, m, d] = text.split('-').map(Number); return new Date(y, m - 1, d); };
const longDay = (text: string) => parse(text).toLocaleDateString('pt-BR', { weekday: 'long', day: 'numeric', month: 'long' });

/** Segunda-feira da primeira semana do mês de `month` (início da grade de seis semanas). */
function monthStart(month: Date): Date {
  const first = new Date(month.getFullYear(), month.getMonth(), 1);
  return new Date(first.getFullYear(), first.getMonth(), 1 - ((first.getDay() + 6) % 7));
}
/** Seis semanas (segunda a domingo) a partir de `start`. */
function weeksFrom(start: Date): string[] {
  return Array.from({ length: 42 }, (_, i) => iso(new Date(start.getFullYear(), start.getMonth(), start.getDate() + i)));
}
/** Quanto a roda precisa andar para descer uma semana (o touchpad manda passos pequenos). */
const WHEEL_STEP = 60;

/** Calendário da Athena: notas do dia (só as da Athena), eventos e lembretes, com repetições. */
/** Campo de data ou hora com máscara que deixa o cursor onde a pessoa clicou ou digitou. */
function MaskedInput({ value, max, format, onValue, ...rest }: { value: string; max: number; format: (digits: string) => string; onValue: (value: string) => void } & Omit<InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange' | 'max'>) {
  const input = useRef<HTMLInputElement>(null);
  const caret = useRef<number | null>(null);
  useLayoutEffect(() => {
    if (caret.current !== null && input.current && document.activeElement === input.current) input.current.setSelectionRange(caret.current, caret.current);
    caret.current = null;
  });
  return <input {...rest} ref={input} inputMode="numeric" value={value} onChange={e => {
    const next = maskedEdit(value, e.target.value, e.target.selectionStart ?? e.target.value.length, max, format);
    caret.current = next.caret;
    // Mesmo valor (ex.: dígito a mais num campo cheio no fim): o React não redesenha, então o cursor volta aqui.
    if (next.value === value) { e.target.value = value; e.target.setSelectionRange(next.caret, next.caret); }
    onValue(next.value);
  }} />;
}

export default function AthenaCalendar({ onClose, onOpenDay }: { onClose: () => void; onOpenDay: (date: string) => void }) {
  const today = iso(new Date());
  // A grade começa numa segunda e mostra seis semanas; a roda do mouse desce ou sobe uma semana.
  const [start, setStart] = useState(() => monthStart(new Date()));
  const [selected, setSelected] = useState(today);
  const [data, setData] = useState<Range>({ items: [], notes: [] });
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState('');
  const days = useMemo(() => weeksFrom(start), [start]);
  // O mês do título é o do meio da grade (o que ocupa a maior parte dela).
  const month = useMemo(() => { const mid = parse(days[20]); return new Date(mid.getFullYear(), mid.getMonth(), 1); }, [days]);
  const grid = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = grid.current;
    if (!el) return;
    let acc = 0;
    const wheel = (e: WheelEvent) => {
      if (e.ctrlKey || e.metaKey || Math.abs(e.deltaY) < Math.abs(e.deltaX)) return;
      e.preventDefault();
      // Roda com "linhas" (mouse comum) vale um passo por dente.
      acc += e.deltaMode === 1 ? Math.sign(e.deltaY) * WHEEL_STEP : e.deltaY;
      const weeks = Math.trunc(acc / WHEEL_STEP);
      if (!weeks) return;
      acc -= weeks * WHEEL_STEP;
      setStart(s => new Date(s.getFullYear(), s.getMonth(), s.getDate() + weeks * 7));
    };
    el.addEventListener('wheel', wheel, { passive: false });
    return () => el.removeEventListener('wheel', wheel);
  }, []);

  const load = useCallback(() => {
    void native<Range>('calendar_range', { from: days[0], to: days[days.length - 1] }).then(setData).catch(e => setError(errorText(e)));
  }, [days]);
  useEffect(() => {
    load();
    const stop = listen('reminders-updated', load);
    // Ocorrências passam a "passadas" com o tempo e a data de hoje muda à meia-noite.
    const timer = setInterval(load, 60_000);
    return () => { void stop.then(off => off()); clearInterval(timer); };
  }, [load]);

  const byDay = useMemo(() => {
    const map = new Map<string, Item[]>();
    for (const item of data.items) map.set(item.date, [...(map.get(item.date) ?? []), item]);
    return map;
  }, [data.items]);
  const notes = useMemo(() => new Set(data.notes), [data.notes]);
  const step = (delta: number) => setStart(monthStart(new Date(month.getFullYear(), month.getMonth() + delta, 1)));
  const goToday = () => { setStart(monthStart(new Date())); setSelected(today); };
  // A grade é contínua: escolher um dia não pula de mês (ele já está à vista).
  const pick = (date: string) => { setSelected(date); setDraft(null); };

  const newDraft = (reminder: boolean, date = selected): Draft => ({ id: null, text: '', date: showDate(date), allDay: !reminder, time: reminder ? '09:00' : '', repeat: 'none', alerts: reminder ? [{ unit: 'na-hora', amount: 1 }] : [] });
  const edit = (item: Item) => setDraft({ id: item.id, text: item.text, date: showDate(item.start_date), allDay: !item.time, time: item.time ?? '', repeat: item.repeat, alerts: [...item.alerts].sort((a, b) => b - a).map(alertFrom) });
  const setAlert = (index: number, change: Partial<Alert>) => draft && setDraft({ ...draft, alerts: draft.alerts.map((a, k) => k === index ? { ...a, ...change } : a) });
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!draft) return;
    setError('');
    const date = parseDate(draft.date);
    const time = draft.allDay ? null : parseTime(draft.time);
    const alerts = draft.alerts.map(alertMinutes);
    if (!date) { setError('Data inválida: use DD/MM/AAAA, por exemplo 25/12/2026.'); return; }
    if (!draft.allDay && !time) { setError('Hora inválida: use o formato de 24 horas, por exemplo 15:30.'); return; }
    if (alerts.some(m => m === null)) { setError('Aviso inválido: informe um número inteiro maior que zero (até 365 dias).'); return; }
    try {
      await native('calendar_save', { id: draft.id, text: draft.text, date, time, repeat: draft.repeat, alerts });
      setDraft(null); load();
    } catch (e) { setError(errorText(e)); }
  }
  const remove = (id: string) => { setError(''); void native('reminder_delete', { id }).then(load).catch(e => setError(errorText(e))); };

  const monthName = month.toLocaleDateString('pt-BR', { month: 'long', year: 'numeric' });
  const label = monthName.charAt(0).toUpperCase() + monthName.slice(1);
  const dayItems = byDay.get(selected) ?? [];
  return <section className="athena-calendar" aria-label="Calendário">
    <header className="organ-bar calendar-bar">
      <button className="icon-button" aria-label="Mês anterior" data-tip="Mês anterior" onClick={() => step(-1)}><ChevronLeftIcon size={18} /></button>
      <h2 className="calendar-title">{label}</h2>
      <button className="icon-button" aria-label="Próximo mês" data-tip="Próximo mês" onClick={() => step(1)}><ChevronRightIcon size={18} /></button>
      <button className="ghost-button" onClick={goToday}>Hoje</button>
      <span className="calendar-spacer" />
      <button className="icon-button tip-end" aria-label="Fechar o calendário" data-tip="Voltar às notas" onClick={onClose}><CloseIcon size={17} /></button>
    </header>
    {error && <p role="alert" className="app-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
    <div className="calendar-body">
      <div className="calendar-month" role="grid" aria-label={label} ref={grid}>
        {WEEK.map(name => <span key={name} className="calendar-weekday" role="columnheader">{name}</span>)}
        {days.map(date => {
          const items = byDay.get(date) ?? [];
          const outside = parse(date).getMonth() !== month.getMonth();
          return <button key={date} role="gridcell" aria-selected={date === selected} aria-current={date === today ? 'date' : undefined}
            className={['calendar-day', outside ? 'outside' : '', date === selected ? 'selected' : '', date === today ? 'today' : ''].join(' ')}
            aria-label={`${longDay(date)}${notes.has(date) ? ', com nota do dia' : ''}${items.length ? `, ${items.length} ${items.length === 1 ? 'item' : 'itens'}` : ''}`}
            onClick={() => pick(date)} onDoubleClick={() => { pick(date); setDraft(newDraft(false, date)); }}>
            <span className="calendar-number">{parse(date).getDate()}{notes.has(date) && <FileIcon size={11} />}</span>
            {items.slice(0, 3).map((item, k) => <span key={`${item.id}-${k}`} className={['calendar-chip', item.notify ? 'reminder' : '', item.past ? 'past' : ''].join(' ')}>{item.time && <b>{item.time}</b>}{item.text}</span>)}
            {items.length > 3 && <span className="calendar-more">+{items.length - 3}</span>}
          </button>;
        })}
      </div>

      <aside className="calendar-day-panel" aria-label={longDay(selected)}>
        <h3>{longDay(selected)}</h3>
        <button className="calendar-note" onClick={() => onOpenDay(selected)}>
          <FileIcon size={15} />{notes.has(selected) ? 'Abrir a nota do dia' : 'Criar a nota do dia'}
        </button>
        <ul className="calendar-items">
          {!dayItems.length && <li className="panel-muted">Nada marcado neste dia.</li>}
          {dayItems.map((item, k) => <li key={`${item.id}-${k}`} className={item.past ? 'past' : ''}>
            <span className="calendar-when">{item.time ?? 'Dia todo'}</span>
            <span className="calendar-what"><b>{item.text}</b><small>{[item.notify ? `Aviso ${alertsLabel(item.alerts)}` : 'Evento sem aviso', item.repeat_label.replace(/[()]/g, '')].filter(Boolean).join(' · ')}</small></span>
            <span className="row-actions">
              <button className="icon-button" aria-label={`Editar ${item.text}`} data-tip="Editar" onClick={() => edit(item)}><PencilIcon size={15} /></button>
              <Confirm className="ghost-button" ariaLabel={`Apagar ${item.text}`} question={item.repeat !== 'none' ? 'Apagar este item e todas as repetições?' : 'Apagar este item do calendário?'} confirm="Apagar" onConfirm={() => remove(item.id)}>Apagar</Confirm>
            </span>
          </li>)}
        </ul>
        {!draft && <div className="row-actions calendar-add">
          <button onClick={() => setDraft(newDraft(false))}><PlusIcon size={15} />Evento</button>
          <button onClick={() => setDraft(newDraft(true))}><PlusIcon size={15} />Lembrete</button>
        </div>}
        {draft && <form className="calendar-form" onSubmit={e => void save(e)}>
          <h4>{draft.id ? 'Editar' : draft.alerts.length ? 'Novo lembrete' : 'Novo evento'}</h4>
          <label>O quê<input autoFocus required maxLength={200} value={draft.text} placeholder={draft.alerts.length ? 'Ligar pro médico' : 'Aniversário da Ana'} onChange={e => setDraft({ ...draft, text: e.target.value })} /></label>
          <div className="calendar-row">
            <label>{draft.repeat === 'none' ? 'Dia' : 'Começa em'}<MaskedInput required placeholder="DD/MM/AAAA" value={draft.date} max={8} format={maskDate} onValue={date => setDraft(d => d && { ...d, date })} /></label>
            {!draft.allDay && <label>Hora (24 h)<MaskedInput required placeholder="HH:MM" value={draft.time} max={4} format={maskTime} onValue={time => setDraft(d => d && { ...d, time })} onBlur={() => { const t = parseTime(draft.time); if (t) setDraft({ ...draft, time: t }); }} /></label>}
          </div>
          <label className="calendar-check"><input type="checkbox" checked={draft.allDay} onChange={e => setDraft({ ...draft, allDay: e.target.checked, time: e.target.checked ? '' : draft.time || '09:00' })} />Dia todo</label>
          <label>Repetir<select value={draft.repeat} onChange={e => setDraft({ ...draft, repeat: e.target.value })}>{REPEATS.map(([value, name]) => <option key={value} value={value}>{name}</option>)}</select></label>
          <fieldset className="calendar-alerts">
            <legend>Avisos por notificação{draft.allDay && draft.alerts.length ? ' (dia todo: contados das 9h)' : ''}</legend>
            {!draft.alerts.length && <p className="panel-muted">Sem aviso: só aparece no calendário.</p>}
            {draft.alerts.map((alert, k) => <div key={k} className="calendar-alert">
              <select aria-label={`Aviso ${k + 1}`} value={alert.unit} onChange={e => setAlert(k, { unit: e.target.value as AlertUnit, amount: alert.unit === 'na-hora' ? 1 : alert.amount })}>{UNITS.map(([value, name]) => <option key={value} value={value}>{name}</option>)}</select>
              {alert.unit !== 'na-hora' && <input aria-label={`Quantos ${alert.unit} antes`} type="number" min={1} max={alert.unit === 'dias' ? 365 : alert.unit === 'horas' ? 8760 : 525600} value={Number.isFinite(alert.amount) ? alert.amount : ''} onChange={e => setAlert(k, { amount: e.target.valueAsNumber })} />}
              <button type="button" className="icon-button" aria-label={`Tirar o aviso ${k + 1}`} data-tip="Tirar aviso" onClick={() => setDraft({ ...draft, alerts: draft.alerts.filter((_, i) => i !== k) })}><CloseIcon size={14} /></button>
            </div>)}
            {draft.alerts.length < 5 && <button type="button" className="ghost-button calendar-add-alert" onClick={() => setDraft({ ...draft, alerts: [...draft.alerts, draft.alerts.some(a => a.unit === 'na-hora') ? { unit: 'horas', amount: 1 } : { unit: 'na-hora', amount: 1 }] })}><PlusIcon size={14} />Adicionar aviso</button>}
          </fieldset>
          {draft.id && draft.repeat !== 'none' && <p className="panel-muted">A mudança vale para todas as repetições.</p>}
          <div className="row-actions"><button type="submit" className="primary-action">Salvar</button><button type="button" className="ghost-button" onClick={() => setDraft(null)}>Cancelar</button></div>
        </form>}
        <p className="panel-muted calendar-hint">Clique duas vezes num dia para abrir a nota dele. Lembretes pedidos na conversa também aparecem aqui.</p>
      </aside>
    </div>
  </section>;
}

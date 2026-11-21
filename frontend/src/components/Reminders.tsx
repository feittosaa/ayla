import { useCallback, useEffect, useState } from 'react';
import Confirm from './Confirm';
import { listen } from '@tauri-apps/api/event';
import { errorText, native } from '../services/Desktop';

type Reminder = { id: string; text: string; due: number; repeat: string; label: string };

/** Configurações › Lembretes: os agendados, com adiar e cancelar. */
export default function Reminders() {
  const [list, setList] = useState<Reminder[]>([]); const [error, setError] = useState('');
  const load = useCallback(() => { void native<Reminder[]>('reminders_list').then(setList).catch(e => setError(errorText(e))); }, []);
  useEffect(() => {
    load();
    const stop = listen('reminders-updated', load);
    const timer = setInterval(load, 60_000);
    return () => { void stop.then(off => off()); clearInterval(timer); };
  }, [load]);
  const act = (command: string, args: Record<string, unknown>) => { setError(''); void native(command, args).then(load).catch(e => setError(errorText(e))); };
  return <section id="lembretes" className="settings-group" aria-labelledby="reminders-title"><h2 id="reminders-title">Lembretes</h2>
    <div className="group-card">
      {!list.length && <div className="group-row"><span className="row-text"><span className="row-title">Nenhum lembrete</span><span className="row-hint">Peça na conversa: "me lembra amanhã às 9h de ligar pro médico" ou "me lembra toda segunda às 7h de ir pra academia".</span></span></div>}
      {list.map(r => <div key={r.id} className="group-row">
        <span className="row-text"><span className="row-title">{r.text}</span><span className="row-hint">{r.label}</span></span>
        <span className="row-actions">
          <button className="ghost-button" onClick={() => act('reminder_snooze', { id: r.id, minutes: 60 })}>Adiar 1 h</button>
          <Confirm className="ghost-button" question="Cancelar este lembrete?" confirm="Cancelar lembrete" onConfirm={() => act('reminder_delete', { id: r.id })}>Cancelar</Confirm>
        </span>
      </div>)}
    </div>
    <p className="group-note">O aviso chega por notificação do sistema, também com a janela fechada, enquanto a Ayla estiver em segundo plano. Um lembrete perdido com o computador desligado avisa quando a Ayla abrir.</p>
    {error && <p role="alert" className="app-error">{error}</p>}
  </section>;
}

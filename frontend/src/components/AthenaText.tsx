import { useCallback, useEffect, useRef, useState } from 'react';
import type { Tab } from './AthenaNote';
import { errorText, native } from '../services/Desktop';

/** Arquivo `.txt` aberto numa aba: lido como texto puro (Ver) ou editado com gravação automática
 * (Editar), no mesmo lugar do disco. Grava o que estiver pendente ao trocar de aba ou fechar. */
export default function AthenaText({ tab, revision, onSaved, onStatus, onError, register }: {
  tab: Tab; revision: number; onSaved: () => void; onStatus: (text: string) => void; onError: (text: string) => void;
  register: (flush: (() => Promise<void>) | null) => void;
}) {
  const [content, setContent] = useState<string | null>(null);
  const [failed, setFailed] = useState('');
  const stamp = useRef<number | null>(null);
  const dirty = useRef(false);
  const latest = useRef('');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saving = useRef<Promise<void> | null>(null);
  const events = useRef({ onSaved, onStatus, onError, register });
  useEffect(() => { events.current = { onSaved, onStatus, onError, register }; });

  const read = useCallback(() => native<{ content: string; modified: number }>('athena_read', { root: tab.root, path: tab.path }), [tab.root, tab.path]);
  const apply = (file: { content: string; modified: number }) => { setContent(file.content); latest.current = file.content; stamp.current = file.modified; dirty.current = false; setFailed(''); };
  useEffect(() => {
    if (dirty.current) return;
    let alive = true;
    read().then(file => { if (alive && !dirty.current) apply(file); }, e => { if (alive) setFailed(errorText(e)); });
    return () => { alive = false; };
  }, [read, revision]);

  const save = useCallback(async () => {
    while (saving.current) await saving.current;
    if (!dirty.current) return;
    const text = latest.current;
    const job = (async () => {
      try {
        stamp.current = await native<number>('athena_write', { root: tab.root, path: tab.path, content: text, expected: stamp.current });
        if (latest.current === text) dirty.current = false;
        events.current.onStatus('Salvo'); events.current.onSaved();
      } catch (e) { events.current.onError(errorText(e)); dirty.current = false; await read().then(apply, () => {}); }
    })();
    saving.current = job;
    try { await job; } finally { saving.current = null; }
  }, [tab.root, tab.path, read]);
  const flush = useCallback(async () => {
    if (timer.current) { clearTimeout(timer.current); timer.current = null; }
    await save();
  }, [save]);
  useEffect(() => {
    events.current.register(flush);
    return () => { events.current.register(null); void flush(); };
  }, [flush]);
  useEffect(() => { if (!tab.editing) void flush(); }, [tab.editing, flush]);

  if (failed) return <div className="note-scroll"><article className="note-page"><h1 className="note-title">{tab.path.slice(tab.path.lastIndexOf('/') + 1)}</h1><p role="alert" className="app-error">{failed}</p></article></div>;
  if (content === null) return <div className="note-scroll" aria-busy="true" />;
  return <div className="note-scroll">
    <article className="note-page text-file">
      <h1 className="note-title">{tab.path.slice(tab.path.lastIndexOf('/') + 1)}</h1>
      {tab.editing
        ? <textarea className="text-editor" aria-label="Texto do arquivo" spellCheck value={content} autoFocus
            onChange={e => {
              const text = e.target.value;
              setContent(text); latest.current = text; dirty.current = true;
              events.current.onStatus('Salvando…');
              if (timer.current) clearTimeout(timer.current);
              timer.current = setTimeout(() => { timer.current = null; void save(); }, 700);
            }} />
        : <pre className="text-view">{content || ' '}</pre>}
    </article>
  </div>;
}

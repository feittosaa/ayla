import { useEffect, useState } from 'react';
import { openDesign, type DesignCard } from '../services/Design';
import { errorText } from '../services/Desktop';

type State = { preview?: string | null; error?: string };

/** Designs que a Ayla criou na resposta: prévia, abrir no editor da Iris e salvar em PNG. O rascunho
 * é montado aqui (Fabric.js, carregado só quando aparece um design) e salvo na Iris com a prévia. */
export default function ChatDesigns({ items }: { items: DesignCard[] }) {
  const [states, setStates] = useState<Record<string, State>>({});
  const [note, setNote] = useState('');
  const [saving, setSaving] = useState<string | null>(null);
  const ids = items.map(i => i.id).join(',');

  useEffect(() => {
    let alive = true;
    void (async () => {
      const canvas = await import('../services/DesignCanvas');
      // Um por vez: cada montagem carrega fotos e fontes.
      for (const item of ids.split(',').filter(Boolean)) {
        try {
          const preview = await canvas.ensureDesign(item);
          if (alive) setStates(prev => ({ ...prev, [item]: { preview } }));
        } catch (e) { if (alive) setStates(prev => ({ ...prev, [item]: { error: errorText(e) } })); }
      }
    })();
    return () => { alive = false; };
  }, [ids]);

  async function save(item: DesignCard) {
    setSaving(item.id); setNote('');
    try {
      const { exportDesign } = await import('../services/DesignCanvas');
      const result = await exportDesign(item.id, 'computer');
      if (!result.cancelled) setNote(`Salvo como ${result.name}.`);
    } catch (e) { setNote(errorText(e)); }
    finally { setSaving(null); }
  }

  return <div className="chat-designs" aria-label="Designs criados">
    {items.map(item => {
      const state = states[item.id];
      const gone = state && state.preview === null && !state.error;
      return <figure key={item.id} className="chat-design">
        <button type="button" className="chat-design-preview" style={{ aspectRatio: `${item.width} / ${item.height}` }} disabled={gone} title={gone ? 'Este design foi apagado' : `Abrir ${item.name} no editor`} onClick={() => openDesign(item.id)}>
          {state?.preview ? <img src={state.preview} alt={`Prévia de ${item.name}`} />
            : <span role="status">{state?.error ? 'Não consegui montar a prévia' : gone ? 'Design apagado' : 'Montando…'}</span>}
        </button>
        <figcaption>
          <b>{item.name}</b><small>{item.width} × {item.height} px</small>
          {!gone && <span className="chat-design-actions">
            <button type="button" className="ghost-button" onClick={() => openDesign(item.id)}>Abrir no editor</button>
            <button type="button" className="ghost-button" disabled={saving !== null || !state?.preview} onClick={() => void save(item)}>{saving === item.id ? 'Salvando…' : 'Salvar PNG…'}</button>
          </span>}
        </figcaption>
      </figure>;
    })}
    {note && <p className="chat-design-note" role="status">{note}</p>}
  </div>;
}

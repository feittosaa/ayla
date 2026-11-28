import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { nameOf } from '../services/Hand';
import { errorText, native } from '../services/Desktop';

export type HandWallpaperCard = { id: string; opcoes: string[]; escolhida: number; programa: string; anterior?: string | null };
type Saved = { feito?: string; anterior?: string | null; cancelado?: boolean };

/** Miniatura de uma imagem da pasta pessoal (feita e guardada pela Ayla, neste computador). */
function Thumb({ path }: { path: string }) {
  const [url, setUrl] = useState('');
  useEffect(() => {
    let alive = true; let made = '';
    invoke<ArrayBuffer>('hand_miniatura', { caminho: path }).then(bytes => { if (!alive) return; made = URL.createObjectURL(new Blob([bytes], { type: 'image/jpeg' })); setUrl(made); }).catch(() => {});
    return () => { alive = false; if (made) URL.revokeObjectURL(made); };
  }, [path]);
  return url ? <img src={url} alt="" /> : <span className="hand-thumb-empty" />;
}

/** Papel de parede: as opções (do que a conversa moveu ou achou, ou da pasta citada), uma já
 * marcada; só muda com "Definir", e "Voltar ao anterior" desfaz. */
export default function HandWallpaper({ card }: { card: HandWallpaperCard }) {
  const key = `ayla-hand-fundo-${card.id}`;
  const [saved, setSaved] = useState<Saved>(() => { try { return JSON.parse(localStorage.getItem(key) ?? '{}') as Saved; } catch { return {}; } });
  const [chosen, setChosen] = useState(Math.min(card.escolhida, card.opcoes.length - 1));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const keep = (next: Saved) => { setSaved(next); localStorage.setItem(key, JSON.stringify(next)); };
  async function apply(path: string, back = false) {
    setBusy(true); setError('');
    try {
      const done = await native<{ anterior?: string | null }>('hand_papel_de_parede', { caminho: path });
      keep(back ? { feito: path, anterior: null } : { feito: path, anterior: done.anterior ?? card.anterior ?? null });
    } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  }
  return <div className="hand-wallpaper">
    {card.opcoes.length > 1 && !saved.feito && !saved.cancelado && <div className="hand-thumbs" role="radiogroup" aria-label="Imagem do papel de parede">
      {card.opcoes.map((path, i) => <button key={path} type="button" role="radio" aria-checked={i === chosen} title={nameOf(path)} className={i === chosen ? 'hand-thumb chosen' : 'hand-thumb'} onClick={() => setChosen(i)}><Thumb path={path} /></button>)}
    </div>}
    {(card.opcoes.length === 1 || saved.feito) && <div className="hand-thumb single"><Thumb path={saved.feito ?? card.opcoes[chosen]} /></div>}
    <p className="hand-name"><b>{nameOf(saved.feito ?? card.opcoes[chosen])}</b><small>pelo {card.programa}</small></p>
    {saved.cancelado ? <small className="panel-muted">Cancelado: o papel de parede não mudou.</small>
      : saved.feito ? <div className="row-actions"><span className="hand-done">Papel de parede definido.</span>
        {saved.anterior && saved.anterior !== saved.feito && <button type="button" className="ghost-button" disabled={busy} onClick={() => void apply(saved.anterior as string, true)}>Voltar ao anterior</button>}</div>
      : <div className="row-actions">
        <button type="button" className="primary-action" disabled={busy} onClick={() => void apply(card.opcoes[chosen])}>{busy ? 'Definindo…' : 'Definir'}</button>
        <button type="button" className="ghost-button" disabled={busy} onClick={() => keep({ cancelado: true })}>Cancelar</button>
      </div>}
    {error && <p role="alert" className="app-error">{error}</p>}
  </div>;
}

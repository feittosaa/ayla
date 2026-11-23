import { useEffect, useState } from 'react';
import { errorText, native, type FileInfo, type ImageItem, type ImageResults as Results } from '../services/Desktop';
import { ChevronLeftIcon, ChevronRightIcon, CloseIcon } from './Icons';

/** Domínios das fontes da busca: só deles a Ayla carrega miniaturas. */
const HOSTS = ['images.unsplash.com', 'images.pexels.com', 'pixabay.com', 'cdn.pixabay.com', 'th.wallhaven.cc', 'w.wallhaven.cc'];
const safe = (url: string) => { try { const u = new URL(url); return u.protocol === 'https:' && HOSTS.includes(u.hostname); } catch { return false; } };
const PROVIDER: Record<string, string> = { unsplash: 'Unsplash', pexels: 'Pexels', pixabay: 'Pixabay', wallhaven: 'Wallhaven' };
const openWeb = (url: string) => native('open_web', { url });

/** Grade com as imagens encontradas: licença, autor, abrir grande, salvar no projeto e abrir no site. */
export default function ImageResults({ results, project, onError }: { results: Results; project: string | null; onError: (text: string) => void }) {
  const [index, setIndex] = useState<number | null>(null);
  // Miniatura que não carrega mais (as do Pixabay valem 24 h): fica o aviso e "Abrir no site".
  const [broken, setBroken] = useState<Record<string, boolean>>({});
  const [saved, setSaved] = useState<Record<string, string>>({}); const [saving, setSaving] = useState<string | null>(null);
  const items = results.items.filter(item => safe(item.thumb));
  const open = index !== null ? items[index] ?? null : null;
  const step = (delta: number) => setIndex(i => i === null ? i : (i + delta + items.length) % items.length);
  // Tela grande: ← e → passam as imagens, Esc fecha.
  useEffect(() => {
    if (index === null) return;
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setIndex(null);
      else if (event.key === 'ArrowRight' && items.length > 1) { event.preventDefault(); step(1); }
      else if (event.key === 'ArrowLeft' && items.length > 1) { event.preventDefault(); step(-1); }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
    // `step` só usa o tamanho da lista.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [index, items.length]);
  async function save(item: ImageItem) {
    const key = `${item.provider}:${item.id}`;
    setSaving(key);
    try { const file = await native<FileInfo>('image_save', { item, project }); setSaved(prev => ({ ...prev, [key]: file.name })); }
    catch (e) { onError(errorText(e)); } finally { setSaving(null); }
  }
  const credit = (item: ImageItem) => `${item.author ? `${item.author} · ` : ''}${PROVIDER[item.provider] ?? item.provider}`;
  const actions = (item: ImageItem) => {
    const key = `${item.provider}:${item.id}`;
    return <span className="image-actions">
      <button type="button" className="ghost-button" disabled={saving === key || Boolean(saved[key])} onClick={() => void save(item)}>{saved[key] ? 'Salva' : saving === key ? 'Salvando…' : 'Salvar'}</button>
      <button type="button" className="ghost-button" onClick={() => void openWeb(item.page).catch(e => onError(errorText(e)))}>Abrir no site</button>
    </span>;
  };
  return <div className="image-results">
    {items.length > 0 && <div className="image-grid">
      {items.map((item, i) => <figure key={`${item.provider}:${item.id}`} className="image-card">
        <button type="button" className="image-thumb" aria-label={`Ver imagem de ${credit(item)}`} onClick={() => setIndex(i)}>{broken[`${item.provider}:${item.id}`]
          ? <span className="image-expired">Prévia indisponível</span>
          : <img src={item.thumb} alt="" loading="lazy" referrerPolicy="no-referrer" onError={() => setBroken(prev => ({ ...prev, [`${item.provider}:${item.id}`]: true }))} />}</button>
        <figcaption>
          <span className={item.license === 'livre' ? 'license-badge free' : 'license-badge'} title={item.license_name}>{item.license === 'livre' ? 'Livre para usar' : 'Só referência'}</span>
          <span className="image-credit">{credit(item)}</span>
          {actions(item)}
        </figcaption>
      </figure>)}
    </div>}
    {results.links.length > 0 && <p className="image-links"><span>Ver também</span>{results.links.map(link => <button key={link.name} type="button" className="link-button" onClick={() => void openWeb(link.url).catch(e => onError(errorText(e)))}>{link.name}</button>)}</p>}
    {open && index !== null && <div className="image-lightbox" role="dialog" aria-modal="true" aria-label={credit(open)} onClick={() => setIndex(null)}>
      <div className="image-lightbox-bar" onClick={e => e.stopPropagation()}>
        <span><b>{credit(open)}</b><small>{open.width && open.height ? `${open.width}×${open.height} · ` : ''}<button type="button" className="link-button" onClick={() => void openWeb(open.license_url).catch(e => onError(errorText(e)))}>{open.license_name}</button></small></span>
        {items.length > 1 && <span className="image-count">{index + 1} de {items.length}</span>}
        {actions(open)}
        <button type="button" className="icon-button" aria-label="Fechar (Esc)" autoFocus onClick={() => setIndex(null)}><CloseIcon size={18} /></button>
      </div>
      <img key={`${open.provider}:${open.id}`} src={safe(open.full) ? open.full : open.thumb} alt="" referrerPolicy="no-referrer" onClick={e => e.stopPropagation()} />
      {items.length > 1 && <>
        <button type="button" className="iris-nav prev" aria-label="Anterior (←)" onClick={e => { e.stopPropagation(); step(-1); }}><ChevronLeftIcon size={26} /></button>
        <button type="button" className="iris-nav next" aria-label="Próxima (→)" onClick={e => { e.stopPropagation(); step(1); }}><ChevronRightIcon size={26} /></button>
      </>}
    </div>}
  </div>;
}

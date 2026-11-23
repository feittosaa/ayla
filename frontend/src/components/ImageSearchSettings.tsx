import { useEffect, useState } from 'react';
import Confirm from './Confirm';
import { errorText, native } from '../services/Desktop';
import { basesChanged } from '../services/Bases';

const PROVIDERS = [
  { id: 'unsplash', name: 'Unsplash', get: 'https://unsplash.com/developers', hint: 'Access Key de um app em unsplash.com/developers.' },
  { id: 'pexels', name: 'Pexels', get: 'https://www.pexels.com/api/', hint: 'Chave da API em pexels.com/api.' },
  { id: 'pixabay', name: 'Pixabay', get: 'https://pixabay.com/api/docs/', hint: 'Chave que aparece em pixabay.com/api/docs com a conta aberta.' },
] as const;
type Status = Record<(typeof PROVIDERS)[number]['id'], boolean>;

/** Chaves gratuitas das fontes de imagens livres, guardadas no cofre do sistema. */
export default function ImageSearchSettings() {
  const [status, setStatus] = useState<Status | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({}); const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const load = () => void native<Status>('image_keys_status').then(setStatus).catch(e => setError(errorText(e)));
  useEffect(load, []);
  async function save(id: string, key: string | null, done: string) {
    setError(''); setNotice('');
    try { await native('image_key_set', { provider: id, key }); setDrafts(prev => ({ ...prev, [id]: '' })); setNotice(done); load(); basesChanged(); } catch (e) { setError(errorText(e)); }
  }
  return <section id="imagens" className="settings-group" aria-labelledby="images-title"><h2 id="images-title">Busca de imagens e pesquisa acadêmica</h2>
    <div className="group-card">
      {PROVIDERS.map(p => <div key={p.id} className="group-row">
        <span className="row-text"><span className="row-title">{p.name}</span>
          <span className="row-hint">{status?.[p.id] ? 'Chave guardada no cofre do sistema.' : <>{p.hint} <button type="button" className="link-button" onClick={() => void native('open_web', { url: p.get }).catch(e => setError(errorText(e)))}>Pegar a chave</button></>}</span></span>
        {status?.[p.id]
          ? <Confirm className="ghost-button" question={`Apagar a chave do ${p.name}?`} confirm="Apagar" onConfirm={() => void save(p.id, null, `Chave do ${p.name} apagada.`)}>Apagar</Confirm>
          : <form className="row-actions" onSubmit={e => { e.preventDefault(); if (drafts[p.id]?.trim()) void save(p.id, drafts[p.id], `Chave do ${p.name} guardada.`); }}>
              <input type="password" autoComplete="off" aria-label={`Chave do ${p.name}`} placeholder="Chave" value={drafts[p.id] ?? ''} onChange={e => setDrafts(prev => ({ ...prev, [p.id]: e.target.value }))} />
              <button type="submit" disabled={!drafts[p.id]?.trim()}>Guardar</button>
            </form>}
      </div>)}
    </div>
    <p className="group-note">Peça na conversa, por exemplo "busca imagens de montanhas para uma apresentação", "um wallpaper de cidade à noite" ou "artigos científicos sobre reforma tributária". Só os termos da busca saem do computador. Unsplash, Pexels e Pixabay têm licença livre (apresentações, mockups e uso comercial); o Wallhaven, que não pede chave, serve de referência e papel de parede pessoal. Pinterest, Cosmos, same.energy e Are.na aparecem como links. Artigos vêm do OpenAlex, Crossref e arXiv, com link para o Google Acadêmico.</p>
    {error && <p role="alert" className="app-error">{error}</p>}
    {notice && <p role="status" className="notice">{notice}</p>}
  </section>;
}

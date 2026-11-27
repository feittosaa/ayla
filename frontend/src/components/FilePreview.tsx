import { useEffect, useState } from 'react';
import ImageLightbox from './ImageLightbox';
import { native, errorText, type FileInfo } from '../services/Desktop';
import { DownloadIcon, FileIcon } from './Icons';

/** Imagem guardada pela Ayla, exibida na conversa, com "Salvar" para copiar para o computador. */
export default function FilePreview({ file }: { file: FileInfo }) {
  const [url, setUrl] = useState<string | null>(null); const [error, setError] = useState('');
  const [zoomed, setZoomed] = useState(false);
  useEffect(() => {
    let alive = true;
    native<{ url: string }>('file_data', { id: file.id }).then(result => { if (alive) setUrl(result.url); }).catch(e => { if (alive) setError(errorText(e)); });
    return () => { alive = false; };
  }, [file.id]);
  if (error) return <span className="file-chip"><FileIcon size={14} />{file.name}</span>;
  return <figure className="chat-image">
    {url ? <button type="button" className="chat-image-open" title="Clique para ampliar" onClick={() => setZoomed(true)}><img src={url} alt={file.name} /></button> : <span className="chat-image-loading" role="status">Carregando imagem…</span>}
    {url && zoomed && <ImageLightbox src={url} alt={file.name} caption="Ctrl + roda para aproximar" onClose={() => setZoomed(false)} />}
    <figcaption><span>{file.name}</span><button type="button" className="ghost-button" onClick={() => void native('export_file', { id: file.id }).catch(e => setError(errorText(e)))}><DownloadIcon size={14} />Salvar</button></figcaption>
  </figure>;
}

/** Miniatura de uma imagem anexada, no campo de mensagem, antes de enviar. */
export function AttachmentThumb({ file }: { file: FileInfo }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    native<{ url: string }>('file_data', { id: file.id }).then(result => { if (alive) setUrl(result.url); }).catch(() => {});
    return () => { alive = false; };
  }, [file.id]);
  return url ? <img className="file-thumb" src={url} alt="" /> : <FileIcon size={14} />;
}

import { useEffect, useState } from 'react';
import { native, errorText, type FileInfo } from '../services/Desktop';
import { DownloadIcon, FileIcon } from './Icons';

/** Imagem guardada pela Ayla, exibida na conversa, com "Salvar" para copiar para o computador. */
export default function FilePreview({ file }: { file: FileInfo }) {
  const [url, setUrl] = useState<string | null>(null); const [error, setError] = useState('');
  useEffect(() => {
    let alive = true;
    native<{ url: string }>('file_data', { id: file.id }).then(result => { if (alive) setUrl(result.url); }).catch(e => { if (alive) setError(errorText(e)); });
    return () => { alive = false; };
  }, [file.id]);
  if (error) return <span className="file-chip"><FileIcon size={14} />{file.name}</span>;
  return <figure className="chat-image">
    {url ? <img src={url} alt={file.name} /> : <span className="chat-image-loading" role="status">Carregando imagem…</span>}
    <figcaption><span>{file.name}</span><button type="button" className="ghost-button" onClick={() => void native('export_file', { id: file.id }).catch(e => setError(errorText(e)))}><DownloadIcon size={14} />Salvar</button></figcaption>
  </figure>;
}

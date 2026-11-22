import type { ReactNode } from 'react';
import PdfView from './PdfView';
import Video from './Video';
import ZoomImage from './ZoomImage';
import { errorText } from '../services/Desktop';
import { mediaKey, mediaKind, mediaUrl, openInSystem, type MediaKind, type MediaRef } from '../services/Media';

/** Leitor de mídia comum à Athena e à Iris: imagem com zoom (Ctrl + roda) e arrastar, vídeo com
 * controles e PDF no leitor da Ayla. Com `bar`, mostra o nome e "Abrir no visualizador do
 * sistema" (no PDF, o botão vai para a barra do próprio leitor). */
export default function MediaView({ media, name, kind, bar, autoPlay, actions, onError }: {
  media: MediaRef; name: string; kind?: MediaKind; bar?: boolean; autoPlay?: boolean; actions?: ReactNode; onError?: (text: string) => void;
}) {
  const src = mediaUrl(media); const key = mediaKey(media);
  const system = () => void openInSystem(media).catch(e => onError?.(errorText(e)));
  const systemButton = <button type="button" className="ghost-button" onClick={system}>Abrir no visualizador do sistema</button>;
  const type = kind ?? mediaKind(name);
  if (type === 'pdf') return <div className="media-view pdf"><PdfView key={key} src={src} onSystem={system} actions={<>{actions}{bar && systemButton}</>} /></div>;
  return <div className="media-view">
    {bar && <div className="attachment-bar"><b>{name}</b><span className="row-actions">{actions}{systemButton}</span></div>}
    <div className="media-stage">{type === 'video' ? <Video key={key} src={src} autoPlay={autoPlay} onSystem={system} /> : <ZoomImage key={key} src={src} alt={name} />}</div>
  </div>;
}

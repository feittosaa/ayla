import { native } from './Desktop';
import { PDF, VIDEO, attachmentUrl, storedUrl } from './Wiki';

/** Serviço único de mídia da Athena e da Iris: o mesmo endereço, o mesmo leitor (MediaView) e o
 * mesmo "abrir no sistema" para imagens, vídeos e PDFs, venham de um cofre ou do chat. */
export type MediaKind = 'image' | 'video' | 'pdf';
/** Arquivo de uma raiz da Athena (cofre, notas, projeto) ou guardado na Ayla (anexo, imagem gerada). */
export type MediaRef = { root: string; path: string } | { file: string };

export const mediaKind = (name: string): MediaKind => PDF.test(name) ? 'pdf' : VIDEO.test(name) ? 'video' : 'image';
/** Endereço servido pelo núcleo (com `thumb`, a miniatura da imagem ou da 1ª página do PDF). */
export const mediaUrl = (media: MediaRef, thumb = false) => 'file' in media ? storedUrl(media.file, thumb) : attachmentUrl(media.root, media.path, thumb);
export const mediaKey = (media: MediaRef) => 'file' in media ? `arquivo:${media.file}` : `${media.root}|${media.path}`;
/** Abre no aplicativo padrão do sistema. */
export const openInSystem = (media: MediaRef) => native<void>('media_open', 'file' in media ? { file: media.file } : { root: media.root, path: media.path });

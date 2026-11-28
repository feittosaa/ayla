/** Título (nome sem `.md`) e pasta de um caminho relativo de nota. */
export const titleOf = (path: string) => path.slice(path.lastIndexOf('/') + 1).replace(/\.md$/i, '');
export const folderOf = (path: string) => path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';

/** Prefixos dos links internos (nunca são endereços da web): notas/anexos e tags. */
export const WIKI = 'athena-link:';
export const TAG = 'athena-tag:';
export const IMAGE = /\.(png|jpe?g|jfif|gif|webp|bmp|svg|avif)$/i;
export const PDF = /\.pdf$/i;
export const VIDEO = /\.(mp4|m4v|webm|mov|ogv|mkv)$/i;
/** Anexos que a Athena mostra: imagens, vídeos e PDF (o resto continua link). */
export const isAttachment = (target: string) => { const clean = target.split('#')[0].trim(); return IMAGE.test(clean) || PDF.test(clean) || VIDEO.test(clean); };
const windows = () => typeof navigator !== 'undefined' && navigator.userAgent.includes('Windows');
/** Endereço de um arquivo guardado na Ayla (anexo do chat ou imagem gerada), pelo id. */
export const storedUrl = (id: string, thumb = false) => `${windows() ? 'http://iris-file.localhost/' : 'iris-file://localhost/'}${encodeURIComponent(id)}${thumb ? '?miniatura' : ''}`;
/** Data de alteração conhecida de cada anexo (pelo caminho e pelo nome do arquivo): entra no endereço,
 * para a imagem trocada no lugar (recorte na Iris, outro programa) não continuar a antiga do cache. */
const mediaVersions = new Map<string, number>();
const versionKey = (root: string, target: string) => `${root}|${target.split('#')[0].trim().toLowerCase()}`;
export function setMediaVersions(root: string, files: { path: string; modified: number }[]) {
  for (const file of files) {
    const name = file.path.slice(file.path.lastIndexOf('/') + 1);
    for (const key of [versionKey(root, file.path), versionKey(root, name)]) mediaVersions.set(key, Math.max(mediaVersions.get(key) ?? 0, file.modified));
  }
}
export const mediaVersion = (root: string, target: string) => mediaVersions.get(versionKey(root, target));
/** Endereço de um anexo de uma raiz (servido pelo núcleo, lido da pasta da nota). */
export const attachmentUrl = (root: string, target: string, thumb = false) => {
  const version = mediaVersion(root, target);
  const query = [thumb ? 'miniatura' : '', version ? `v=${version}` : ''].filter(Boolean).join('&');
  return `${windows() ? 'http://athena-file.localhost/' : 'athena-file://localhost/'}${encodeURIComponent(root)}/${encodeURIComponent(target)}${query ? `?${query}` : ''}`;
};

/** Sintaxe do Obsidian em Markdown comum: [[nota]], [[nota|apelido]], [[nota#seção]], ![[foto.png]]
 * e ![[foto.png|300]] (imagem, com largura), ![[livro.pdf]] (PDF na nota) e #tag. Código em bloco ou em linha fica como está. */
export function wikiToMarkdown(text: string, embed?: (name: string) => string | undefined) {
  return text.split(/(```[\s\S]*?```|`[^`\n]*`)/g).map((part, i) => i % 2 ? part : part
    .replace(/(!?)\[\[([^\]\n|#]+)(#[^\]\n|]*)?(?:\|([^\]\n]+))?\]\]/g, (_match: string, bang: string, target: string, section: string | undefined, alias: string | undefined) => {
      const typed = target.trim();
      const clean = (bang && embed?.(typed)) || typed;
      // ![[foto.png|300]] ou |300x200: tamanho, como no Obsidian (não é apelido).
      const size = bang && alias && /^\d+(x\d+)?$/.test(alias.trim()) ? alias.trim() : '';
      const label = (size ? typed : alias || `${typed}${section || ''}`).replace(/[[\]"]/g, '');
      return bang && isAttachment(clean) ? `![${label}](${WIKI}${encodeURIComponent(clean)}${size ? ` "${size}"` : ''})` : `[${label}](${WIKI}${encodeURIComponent(clean)})`;
    })
    .replace(/(^|[ \t])#([\p{L}\p{N}_/-]*\p{L}[\p{L}\p{N}_/-]*)/gmu, (_match: string, space: string, tag: string) => `${space}[#${tag}](${TAG}${encodeURIComponent(tag.toLowerCase())})`)
  ).join('');
}

/** `![[Casco]]` sem extensão: o anexo da raiz com esse nome (imagem primeiro), se houver um só.
 * O Obsidian pede a extensão; aqui o nome basta, para a imagem aparecer como se espera. */
export function embedResolver(files: { path: string }[]) {
  return (name: string) => {
    if (isAttachment(name)) return undefined;
    const wanted = name.trim().toLowerCase();
    const stem = (path: string) => path.replace(/\.[^./]+$/, '').toLowerCase();
    const found = files.filter(f => isAttachment(f.path) && (stem(f.path) === wanted || stem(f.path.slice(f.path.lastIndexOf('/') + 1)) === wanted));
    const images = found.filter(f => IMAGE.test(f.path));
    const pick = images.length ? images : found;
    return pick.length === 1 ? pick[0].path : undefined;
  };
}

export function imageWidth(title?: string) {
  return /^(\d+(?:x\d+)?)$/.exec(title ?? '')?.[1] ?? / \[largura=(\d+)\]$/.exec(title ?? '')?.[1];
}

export function resizedImage(original: string, src: string, alt: string, title: string | undefined, width: number) {
  const wiki = /^!\[\[([^|\]]+)(?:\|[^\]]*)?\]\]$/.exec(original);
  if (wiki) return `![[${wiki[1]}|${width}]]`;
  const label = alt.replace(/[\\[\]]/g, '\\$&');
  const caption = title && !/^\d+(?:x\d+)?$/.test(title) ? title.replace(/ \[largura=\d+\]$/, '') : '';
  const nextTitle = (caption ? `${caption} [largura=${width}]` : String(width)).replace(/[\\"]/g, '\\$&');
  return `![${label}](<${src.replace(/[<>]/g, char => encodeURIComponent(char))}> "${nextTitle}")`;
}

export type Property = { key: string; values: string[] };
export type Banner = { target: string; y: number; height?: number };
export type Frontmatter = { properties: Property[]; aliases: string[]; banner: Banner | null; body: string };

const unquote = (value: string) => value.trim().replace(/^(["'])(.*)\1$/, '$2').trim();
const frontmatterBlock = /^(---[ \t]*(\r?\n))([\s\S]*?)(^---[ \t]*(?:\r?\n|$))/m;
/** Linha que cabe em propriedades YAML: `chave: valor`, item de lista, continuação recuada,
 * comentário ou vazia. Um texto que só começa com `---` (um separador) não vira propriedades. */
const yamlLine = (line: string) => !line.trim() || /^\s*#/.test(line) || /^[^\s:#-][^:]*:(\s|$)/.test(line) || /^\s+\S/.test(line) || /^-(\s|$)/.test(line);
/** O bloco `---` do início da nota, se for mesmo de propriedades. */
function propertiesBlock(text: string) {
  const block = frontmatterBlock.exec(text);
  return block && block.index === 0 && block[3].split(/\r?\n/).every(yamlLine) ? block : null;
}

export function changeBanner(text: string, key: 'banner_height' | 'banner_y', value: number) {
  const block = propertiesBlock(text);
  if (!block) return text;
  const [, opening, newline, properties, closing] = block;
  const field = new RegExp(`^${key}:[^\\r\\n]*`, 'm');
  // Como o make-md: posição em porcentagem (0–100), gravada entre aspas.
  const line = key === 'banner_y' ? `${key}: "${Math.round(Math.min(100, Math.max(0, value)) * 10) / 10}"` : `${key}: ${value}`;
  const updated = field.test(properties) ? properties.replace(field, line) : properties + line + newline;
  return opening + updated + closing + text.slice(block[0].length);
}

const BANNER_KEYS = ['banner', 'banner_y', 'banner_height', 'banner_x', 'banner_icon', 'banner_lock'];
/** Tira o banner do topo da nota (por padrão, `banner` e os ajustes dele, como o make-md grava);
 * com `keys`, só essas chaves. Sem outras propriedades, o bloco `---` sai junto. */
export function removeBanner(text: string, keys: string[] = BANNER_KEYS) {
  const block = propertiesBlock(text);
  if (!block) return text;
  const [, opening, newline, properties, closing] = block;
  const lines = properties.split(/\r?\n/);
  if (lines[lines.length - 1] === '') lines.pop();
  const kept: string[] = []; let skipping = false;
  for (const line of lines) {
    const key = /^([^\s:#-][^:]*):/.exec(line)?.[1].trim();
    if (key !== undefined) skipping = keys.includes(key);
    else if (skipping && !/^(\s|-)/.test(line)) skipping = false;
    if (!skipping) kept.push(line);
  }
  const rest = text.slice(block[0].length);
  if (!kept.some(line => line.trim())) return rest.replace(/^\r?\n/, '');
  return opening + kept.map(line => line + newline).join('') + closing + rest;
}

/** Apelidos da nota (`aliases`, ou `alias` antigo), como o Obsidian os lê. */
export function noteAliases(text: string) {
  const seen = new Set<string>();
  return frontmatter(text).aliases.filter(alias => { const key = alias.toLowerCase(); if (seen.has(key)) return false; seen.add(key); return true; });
}

/** Grava os apelidos no topo da nota como lista do Obsidian (`aliases:` com `  - item`), no lugar de
 * `aliases`/`alias` que já existiam; sem apelidos, a chave sai. Cria o bloco de propriedades se
 * faltar e mantém as outras propriedades e o fim de linha da nota. */
export function setAliases(text: string, aliases: string[]) {
  const clean = [...new Set(aliases.map(a => a.replace(/[\r\n]+/g, ' ').trim()).filter(Boolean))];
  const quote = (value: string) => /^[\w\u00C0-\u024F][\w\u00C0-\u024F .'()-]*$/.test(value) && !/^(true|false|null|yes|no|~)$/i.test(value) ? value : JSON.stringify(value);
  const block = propertiesBlock(text);
  const newline = block?.[2] ?? (text.includes('\r\n') ? '\r\n' : '\n');
  const list = clean.length ? `aliases:${newline}${clean.map(a => `  - ${quote(a)}`).join(newline)}${newline}` : '';
  if (!block) return clean.length ? `---${newline}${list}---${newline}${text}` : text;
  const [, opening, , properties, closing] = block;
  const lines = properties.split(/\r?\n/);
  if (lines[lines.length - 1] === '') lines.pop();
  const kept: string[] = []; let skipping = false; let at = -1;
  for (const line of lines) {
    const key = /^([^\s:#-][^:]*):/.exec(line)?.[1].trim().toLowerCase();
    if (key !== undefined) skipping = key === 'aliases' || key === 'alias';
    else if (skipping && !/^(\s|-)/.test(line)) skipping = false;
    if (skipping) { if (at < 0) at = kept.length; continue; }
    kept.push(line);
  }
  const rest = kept.map(line => line + newline);
  rest.splice(at < 0 ? 0 : at, 0, list);
  const updated = rest.join('');
  if (!updated.trim()) return text.slice(block[0].length);
  return opening + updated + closing + text.slice(block[0].length);
}

/** Propriedades do Obsidian (frontmatter YAML no início da nota): `chave: valor`, listas
 * (`- item` ou `[a, b]`). O banner segue o make-md: `banner` (caminho na raiz; também [[imagem]] ou
 * ![[imagem]]) e `banner_y` (porcentagem, 0–100). Endereços da web não viram banner. */
export function frontmatter(text: string): Frontmatter {
  const match = propertiesBlock(text);
  if (!match) return { properties: [], aliases: [], banner: null, body: text };
  const properties: Property[] = [];
  for (const line of match[3].split(/\r?\n/)) {
    const item = /^\s*-(?:\s+(.*))?$/.exec(line);
    if (item && properties.length) { const value = unquote(item[1] ?? ''); if (value) properties[properties.length - 1].values.push(value); continue; }
    const pair = /^([^\s:#][^:]*):\s*(.*)$/.exec(line);
    if (!pair) continue;
    const raw = pair[2].trim();
    const values = /^\[.*\]$/.test(raw) && !raw.startsWith('[[') ? raw.slice(1, -1).split(',').map(unquote).filter(Boolean) : raw ? [unquote(raw)] : [];
    properties.push({ key: pair[1].trim(), values });
  }
  const value = (key: string) => properties.find(p => p.key === key)?.values[0] ?? '';
  const target = value('banner').replace(/^!?\[\[([^\]|#]+)(?:[|#][^\]]*)?\]\]$/, '$1').trim();
  let y = Number.parseFloat(value('banner_y'));
  // Versões anteriores da Ayla gravavam uma fração sem aspas (`banner_y: 0.19`); o make-md grava "19".
  if (!Number.isFinite(y)) y = 50; else if (/^banner_y:[ \t]*0?\.\d+[ \t]*$/m.test(match[3])) y *= 100;
  const height = Number.parseInt(value('banner_height'), 10);
  const banner = target && !/^[a-z]+:\/\//i.test(target) ? {
    target, y: Math.min(100, Math.max(0, y)),
    ...(Number.isFinite(height) ? { height: Math.min(600, Math.max(80, height)) } : {}),
  } : null;
  const aliases = properties.filter(p => ['aliases', 'alias'].includes(p.key.toLowerCase())).flatMap(p => p.values);
  return { properties: properties.filter(p => ![...BANNER_KEYS, 'aliases', 'alias'].includes(p.key) && !['aliases', 'alias'].includes(p.key.toLowerCase())), aliases, banner, body: text.slice(match[0].length) };
}

/** Parte dos designs que depende do Fabric.js: montar modelos, gradientes e prévias. Só é
 * carregada quando a pessoa abre Designs (import dinâmico), para não pesar no resto da Ayla. */
import { Circle, FabricImage, FabricObject, Gradient, Rect, StaticCanvas, Textbox, loadSVGFromString, util, type StaticCanvas as Static } from 'fabric';
import { draftOf, exportScale, fontValue, formatOf, gradientCoords, previewScale, type DesignSummary, type Draft, type Fill, type Spec, type Template } from './Design';
import { loadPackFonts } from './DesignFonts';
import { native } from './Desktop';

// Como no Canva (e no Fabric até a versão 6): posição pelo canto de cima à esquerda.
FabricObject.ownDefaults.originX = 'left';
FabricObject.ownDefaults.originY = 'top';
FabricObject.ownDefaults.transparentCorners = false;
FabricObject.ownDefaults.cornerStyle = 'circle';
FabricObject.ownDefaults.cornerColor = '#ffffff';
FabricObject.ownDefaults.cornerStrokeColor = '#7c5cff';
FabricObject.ownDefaults.borderColor = '#7c5cff';
FabricObject.ownDefaults.cornerSize = 12;
FabricObject.ownDefaults.borderScaleFactor = 1.5;
// Tudo pode ser apagado pela borracha dos pincéis (@erase2d/fabric).
(FabricObject.ownDefaults as Record<string, unknown>).erasable = true;

/** Propriedades nossas que precisam ir junto no JSON do projeto. */
export const EXTRA = ['icon', 'credit'];

export function fillFrom(fill: Fill, width: number, height: number) {
  if (typeof fill === 'string') return fill;
  return new Gradient({
    type: 'linear', gradientUnits: 'pixels', coords: gradientCoords(fill.angle, width, height),
    colorStops: [{ offset: 0, color: fill.colors[0] }, { offset: 1, color: fill.colors[1] }],
  });
}

export function objectFrom(spec: Spec): FabricObject {
  switch (spec.type) {
    case 'text':
      return new Textbox(spec.text, {
        left: spec.left, top: spec.top, width: spec.width, fontSize: spec.fontSize, fontFamily: fontValue(spec.font),
        fontWeight: spec.bold ? 'bold' : 'normal', fontStyle: spec.italic ? 'italic' : 'normal', fill: spec.fill,
        textAlign: spec.align ?? 'left', charSpacing: spec.spacing ?? 0, lineHeight: spec.lineHeight ?? 1.16, opacity: spec.opacity ?? 1,
      });
    case 'rect':
      return new Rect({ left: spec.left, top: spec.top, width: spec.width, height: spec.height, fill: fillFrom(spec.fill, spec.width, spec.height),
        rx: spec.radius ?? 0, ry: spec.radius ?? 0, angle: spec.angle ?? 0, opacity: spec.opacity ?? 1 });
    case 'circle':
      return new Circle({ left: spec.left, top: spec.top, radius: spec.radius, fill: fillFrom(spec.fill, spec.radius * 2, spec.radius * 2),
        opacity: spec.opacity ?? 1, stroke: spec.stroke, strokeWidth: spec.strokeWidth ?? 0 });
  }
}

/** Coloca o modelo na página (fundo e elementos). */
export function applyTemplate(canvas: Static, template: Template) {
  const format = formatOf(template.format)!;
  canvas.backgroundColor = fillFrom(template.background, format.width, format.height);
  for (const spec of template.objects) canvas.add(objectFrom(spec));
}

/** Miniatura de um modelo (data URL), com o lado maior de `side` px. */
export function templatePreview(template: Template, side = 360) {
  const format = formatOf(template.format)!;
  const scale = side / Math.max(format.width, format.height);
  const element = document.createElement('canvas');
  const canvas = new StaticCanvas(element, { width: format.width * scale, height: format.height * scale, renderOnAddRemove: false, enableRetinaScaling: false });
  canvas.setZoom(scale);
  applyTemplate(canvas, template);
  canvas.renderAll();
  const url = canvas.toDataURL({ format: 'png', multiplier: 1 });
  void canvas.dispose();
  return url;
}

/** Cria canvas com todo contexto 2D sem GPU (`willReadFrequently`). No WebKitGTK, o canvas acelerado
 * às vezes não é mostrado na tela depois de desenhado (página vazia, traço invisível até soltar). */
export function withSoftwareCanvases<T>(make: () => T): T {
  const getContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, type: string, attributes?: object) {
    return getContext.call(this, type, type === '2d' ? { ...attributes, willReadFrequently: true } : attributes);
  } as typeof getContext;
  try { return make(); } finally { HTMLCanvasElement.prototype.getContext = getContext; }
}

/** Lado maior das imagens guardadas dentro do projeto (o design leva a imagem junto). */
export const MAX_IMAGE_SIDE = 2400;

/** Reduz imagens grandes antes de entrarem no projeto. */
export async function fitImage(url: string): Promise<string> {
  const image = new Image();
  image.src = url;
  await image.decode();
  const side = Math.max(image.naturalWidth, image.naturalHeight);
  if (side <= MAX_IMAGE_SIDE) return url;
  const k = MAX_IMAGE_SIDE / side;
  const canvas = document.createElement('canvas');
  canvas.width = Math.round(image.naturalWidth * k); canvas.height = Math.round(image.naturalHeight * k);
  canvas.getContext('2d')!.drawImage(image, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL(/^data:image\/jpe?g/.test(url) ? 'image/jpeg' : 'image/png', 0.9);
}

/** Partes de um ícone: o Fabric devolve um grupo, ou o próprio caminho quando o SVG tem um só. */
export const iconParts = (o: FabricObject): FabricObject[] => 'getObjects' in o && typeof o.getObjects === 'function' ? (o.getObjects as () => FabricObject[])() : [o];

/** Ícone do pacote (SVG de traço) na cor pedida, como no editor. */
export async function iconFrom(svg: string, color: string): Promise<FabricObject> {
  const { objects, options } = await loadSVGFromString(svg);
  const group = util.groupSVGElements(objects.filter((o): o is NonNullable<typeof o> => Boolean(o)), options);
  iconParts(group).forEach(child => child.set({ stroke: color, fill: child.fill && child.fill !== 'none' ? color : '' }));
  group.set({ icon: true } as Record<string, unknown>);
  return group;
}

/** Monta o rascunho da Ayla no Fabric: devolve o projeto (JSON do editor) e a prévia (PNG em base64).
 * Fotos preenchem a caixa pedida, cortando o excesso pelo centro. */
export async function renderDraft(draft: Draft): Promise<{ data: string; preview: string }> {
  await loadPackFonts();
  const { width, height } = draft;
  const element = document.createElement('canvas');
  const canvas = withSoftwareCanvases(() => new StaticCanvas(element, { width, height, renderOnAddRemove: false, enableRetinaScaling: false }));
  try {
    canvas.backgroundColor = fillFrom(draft.background, width, height);
    for (const spec of draft.objects) {
      try {
        if (spec.type === 'icon') {
          const icon = await iconFrom(spec.svg, spec.color);
          icon.scaleToWidth(spec.size);
          icon.set({ left: spec.left, top: spec.top });
          canvas.add(icon);
        } else if (spec.type === 'image') {
          const image = await FabricImage.fromURL(await fitImage(spec.url));
          const k = Math.max(spec.width / image.width, spec.height / image.height);
          const cropW = spec.width / k, cropH = spec.height / k;
          image.set({ left: spec.left, top: spec.top, cropX: (image.width - cropW) / 2, cropY: (image.height - cropH) / 2, width: cropW, height: cropH, scaleX: k, scaleY: k, opacity: spec.opacity });
          if (spec.radius > 0) image.clipPath = new Rect({ width: cropW, height: cropH, rx: spec.radius / k, ry: spec.radius / k, originX: 'center', originY: 'center' });
          if (spec.credit) image.set({ credit: spec.credit } as Record<string, unknown>);
          canvas.add(image);
        } else canvas.add(objectFrom(spec));
      } catch { /* um elemento que não abre (foto, ícone) fica de fora */ }
    }
    canvas.renderAll();
    const data = JSON.stringify(canvas.toObject(EXTRA));
    const preview = canvas.toDataURL({ format: 'png', multiplier: previewScale(width, height) }).split(',')[1];
    return { data, preview };
  } finally { void canvas.dispose(); }
}

/** Garante que um design criado pela Ayla está pronto (montado e com prévia) e devolve a prévia;
 * `null` se o design não existe mais. Rascunhos são montados e salvos no mesmo id. */
export async function ensureDesign(id: string): Promise<string | null> {
  const listed = await native<{ designs: DesignSummary[] }>('design_list');
  const found = listed.designs.find(d => d.id === id);
  if (!found) return null;
  if (found.preview) return found.preview;
  const file = await native<DesignSummary & { data: string }>('design_read', { id });
  const draft = draftOf(file.data);
  if (!draft) return null;
  const { data, preview } = await renderDraft(draft);
  await native<string>('design_save', { id, name: file.name, width: file.width, height: file.height, data, preview });
  return `data:image/png;base64,${preview}`;
}

/** Exporta um design guardado em PNG (para a Iris ou o computador), sem abrir o editor. */
export async function exportDesign(id: string, target: 'iris' | 'computer') {
  await ensureDesign(id);
  await loadPackFonts();
  const file = await native<DesignSummary & { data: string }>('design_read', { id });
  const element = document.createElement('canvas');
  const canvas = withSoftwareCanvases(() => new StaticCanvas(element, { width: file.width, height: file.height, renderOnAddRemove: false, enableRetinaScaling: false }));
  try {
    await canvas.loadFromJSON(file.data);
    canvas.renderAll();
    const multiplier = exportScale(file.width, file.height, 'png');
    const url = canvas.toDataURL({ format: 'png', multiplier });
    return await native<{ cancelled?: boolean; name?: string; file?: string }>('design_export', {
      name: file.name, format: 'png', image: url.split(',')[1], pixels: [Math.round(file.width * multiplier), Math.round(file.height * multiplier)], page: null, target,
    });
  } finally { void canvas.dispose(); }
}

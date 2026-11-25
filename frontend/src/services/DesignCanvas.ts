/** Parte dos designs que depende do Fabric.js: montar modelos, gradientes e prévias. Só é
 * carregada quando a pessoa abre Designs (import dinâmico), para não pesar no resto da Ayla. */
import { Circle, FabricObject, Gradient, Rect, StaticCanvas, Textbox, type StaticCanvas as Static } from 'fabric';
import { fontValue, formatOf, gradientCoords, type Fill, type Spec, type Template } from './Design';

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

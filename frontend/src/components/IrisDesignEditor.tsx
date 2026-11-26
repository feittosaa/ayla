import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { ActiveSelection, Canvas, Circle, FabricImage, FabricObject, Gradient, Line, PencilBrush, Polygon, Rect, SprayBrush, Textbox, Triangle, cache } from 'fabric';
import { EraserBrush } from '@erase2d/fabric';
import { errorText, native } from '../services/Desktop';
import { mediaUrl, type MediaRef } from '../services/Media';
import { History, exportScale, fontName, fontValue, gradientCoords, pdfPage, previewScale, systemFonts, type Fill, type PackFont, type Template } from '../services/Design';
import { EXTRA, applyTemplate, fillFrom, fitImage, iconFrom, iconParts, withSoftwareCanvases } from '../services/DesignCanvas';
import { loadPackFonts } from '../services/DesignFonts';
import { marginOf, snap, snapLines, type Guides } from '../services/DesignGuides';
import { CloseIcon, DownloadIcon, ImageIcon, TrashIcon, CopyIcon } from './Icons';

export type Opening = { id?: string; name: string; width: number; height: number; data?: string; template?: Template };
type Pack = { installed: boolean; bytes: number; progress: string | null; install: () => void };
type LibraryImage = { key: string; name: string; place: string; file?: string; root?: string | null; path?: string };
type Panel = 'texto' | 'formas' | 'imagens' | 'icones' | 'desenhar' | 'fundo';
type ImageSource = 'iris' | 'computador' | 'online';
type OnlineImage = { provider: string; id: string; thumb: string; author: string; license_name: string; page: string; width: number; height: number };
type Brush = 'caneta' | 'marcador' | 'marca-texto' | 'spray' | 'borracha';
const brushes: [Brush, string][] = [['caneta', 'Caneta'], ['marcador', 'Marcador'], ['marca-texto', 'Marca-texto'], ['spray', 'Spray'], ['borracha', 'Borracha']];
/** Larguras dos painéis laterais, arrastando a borda (guardadas entre aberturas). */
const SIDES_KEY = 'ayla-design-paineis';
const sideLimits = { min: 200, max: 520 };
const clampSidePanel = (n: number) => Math.round(Math.min(sideLimits.max, Math.max(sideLimits.min, n)));
function storedSides() {
  try { const v = JSON.parse(localStorage.getItem(SIDES_KEY) ?? ''); return { left: clampSidePanel(Number(v.left) || 264), right: clampSidePanel(Number(v.right) || 264) }; }
  catch { return { left: 264, right: 264 }; }
}
type Leave = 'close' | null;

const swatches = ['#ffffff', '#f4efe6', '#111111', '#264653', '#2a9d8f', '#e9c46a', '#f4a261', '#e76f51', '#ff3d77', '#7c5cff', '#4cc9f0', '#80ed99'];
const gradients: Fill[] = [
  { angle: 135, colors: ['#ff7a59', '#ff3d77'] }, { angle: 160, colors: ['#1b1446', '#5b2a86'] }, { angle: 120, colors: ['#0f2027', '#2c5364'] },
  { angle: 90, colors: ['#2a9d8f', '#264653'] }, { angle: 45, colors: ['#ffd60a', '#ffc300'] }, { angle: 135, colors: ['#a1c4fd', '#c2e9fb'] },
  { angle: 135, colors: ['#f6d365', '#fda085'] }, { angle: 90, colors: ['#43e97b', '#38f9d7'] },
];
const fillCss = (fill: Fill) => typeof fill === 'string' ? fill : `linear-gradient(${fill.angle + 90}deg, ${fill.colors[0]}, ${fill.colors[1]})`;
const isText = (o: FabricObject | undefined): o is Textbox => o instanceof Textbox;
const isIcon = (o: FabricObject | undefined) => Boolean(o && (o as FabricObject & { icon?: boolean }).icon);
const parts = iconParts;
const solid = (value: unknown) => typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value) ? value : null;
const hex = (value: unknown, fallback = '#000000') => {
  if (typeof value !== 'string') return fallback;
  if (/^#[0-9a-f]{6}$/i.test(value)) return value;
  if (/^#[0-9a-f]{3}$/i.test(value)) return '#' + [...value.slice(1)].map(c => c + c).join('');
  const rgb = /^rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(value);
  return rgb ? '#' + rgb.slice(1, 4).map(n => Number(n).toString(16).padStart(2, '0')).join('') : fallback;
};

function Tool({ label, onClick, children, disabled, active }: { label: string; onClick: () => void; children: ReactNode; disabled?: boolean; active?: boolean }) {
  return <button type="button" className={active ? 'icon-button active' : 'icon-button'} data-tip={label} aria-label={label} aria-pressed={active} disabled={disabled} onClick={onClick}>{children}</button>;
}

/** Editor de designs da Iris, no estilo do Canva: página com texto, formas, imagens e ícones,
 * fundo em cor ou gradiente, camadas, alinhamento, desfazer e exportação em PNG, JPG ou PDF. */
export default function IrisDesignEditor({ opening, pack, onSaved, onExported, onClose }: {
  opening: Opening; pack: Pack; onSaved: () => void; onExported: (message: string) => void; onClose: () => void;
}) {
  const { width, height } = opening;
  const host = useRef<HTMLDivElement>(null);
  const element = useRef<HTMLCanvasElement>(null);
  const canvasRef = useRef<Canvas | null>(null);
  const history = useRef<History | null>(null);
  const restoring = useRef(false);
  const commitQueued = useRef(false);
  const clipboard = useRef<FabricObject | null>(null);
  const [id, setId] = useState(opening.id);
  const [name, setName] = useState(opening.name);
  const [savedName, setSavedName] = useState(opening.name);
  const [saved, setSaved] = useState<string | null>(null);
  const [current, setCurrent] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const [, setTick] = useState(0);
  const refresh = useCallback(() => setTick(n => n + 1), []);
  const [zoom, setZoom] = useState<number | 'fit'>('fit');
  const [box, setBox] = useState({ width: 0, height: 0 });
  const [panel, setPanel] = useState<Panel>('texto');
  const [fonts, setFonts] = useState<PackFont[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState('');
  const [error, setError] = useState('');
  const [exporting, setExporting] = useState(false);
  const [leave, setLeave] = useState<Leave>(null);
  const [images, setImages] = useState<LibraryImage[] | null>(null);
  const [imageQuery, setImageQuery] = useState('');
  const [icons, setIcons] = useState<{ query: string; items: { name: string; svg: string }[]; more: boolean } | null>(null);
  const loadingIcons = useRef(false);
  const iconsEnd = useRef<HTMLButtonElement>(null);
  const [iconQuery, setIconQuery] = useState('');
  const [imageSource, setImageSource] = useState<ImageSource>(() => (localStorage.getItem('ayla-design-imagens') as ImageSource | null) ?? 'iris');
  const [onlineQuery, setOnlineQuery] = useState('');
  /** Resultado por banco: cada um aparece quando chega (`null` = ainda buscando). */
  const [online, setOnline] = useState<{ query: string; banks: { id: string; label: string; items: OnlineImage[] | null; error?: string }[] } | null>(null);
  const [broken, setBroken] = useState<Set<string>>(() => new Set());
  const searchRun = useRef(0);
  const [brush, setBrush] = useState<Brush>('caneta');
  const [brushColor, setBrushColor] = useState('#1d1d2b');
  // Cada pincel guarda a própria espessura, que é a largura real do traço (em px da página).
  const [brushSizes, setBrushSizes] = useState<Record<Brush, number>>(() => {
    const u = Math.min(opening.width, opening.height);
    const px = (k: number, min: number) => Math.max(min, Math.round(u * k));
    return { caneta: px(0.006, 2), marcador: px(0.016, 4), 'marca-texto': px(0.03, 8), spray: px(0.03, 8), borracha: px(0.03, 8) };
  });
  const brushSize = brushSizes[brush];
  const [drawing, setDrawing] = useState(true);
  const [margins, setMargins] = useState(() => localStorage.getItem('ayla-design-margens') !== 'nao');
  const [sides, setSides] = useState(storedSides);
  const guides = useRef<Guides>({ x: [], y: [] });
  const marginsRef = useRef(margins);
  marginsRef.current = margins;
  // Ajustar à tela arredonda para baixo e deixa folga: a página nunca passa da área (senão a
  // barra de rolagem aparece, a área encolhe, a página diminui e a barra some, sem parar).
  const fit = box.width && box.height ? Math.floor(Math.min((box.width - 72) / width, (box.height - 72) / height) * 1000) / 1000 : 0;
  const scale = Math.max(0.05, zoom === 'fit' ? fit : zoom);
  const fitRef = useRef(fit);
  fitRef.current = fit;
  const dirty = ready && (current !== saved || name !== savedName);

  const snapshot = useCallback(() => JSON.stringify(canvasRef.current!.toObject(EXTRA)), []);
  const commit = useCallback(() => {
    if (restoring.current || commitQueued.current) return;
    commitQueued.current = true;
    requestAnimationFrame(() => {
      commitQueued.current = false;
      const canvas = canvasRef.current; const h = history.current;
      if (!canvas || !h || restoring.current) return;
      if (h.push(snapshot())) setNote('');
      setCurrent(h.value);
    });
  }, [snapshot]);

  // Página: cria o canvas, carrega o projeto (ou o modelo) e liga os eventos.
  useEffect(() => {
    // Canvas sem aceleração por GPU: no WebKitGTK, o acelerado às vezes não é mostrado na tela
    // depois de desenhado (a página fica vazia). O contexto criado aqui é o que o Fabric reaproveita.
    // Vale também para o canvas de cima (seleção e traço dos pincéis durante o desenho), que o Fabric
    // cria por conta própria.
    const canvas = withSoftwareCanvases(() => {
      element.current!.getContext('2d');
      return new Canvas(element.current!, { width, height, preserveObjectStacking: true, backgroundColor: '#ffffff', stopContextMenu: true });
    });
    canvasRef.current = canvas;
    let alive = true;
    void (async () => {
      restoring.current = true;
      try {
        if (opening.data) await canvas.loadFromJSON(opening.data);
        else if (opening.template) applyTemplate(canvas, opening.template);
      } catch (e) { setError(`Não foi possível abrir o design: ${errorText(e)}`); }
      restoring.current = false;
      if (!alive) return;
      canvas.renderAll();
      // O WebKitGTK só mostra o canvas novo depois que a camada dele existe: desenha de novo logo
      // depois de abrir (sem isso, a página aparece vazia até a primeira mudança).
      for (const ms of [60, 250, 600]) setTimeout(() => { if (alive) canvas.renderAll(); }, ms);
      const first = snapshot();
      history.current = new History(first);
      setCurrent(first); setSaved(first); setReady(true);
    })();
    const changed = () => { commit(); refresh(); };
    canvas.on('object:modified', changed);
    canvas.on('object:added', changed);
    canvas.on('object:removed', changed);
    canvas.on('text:changed', changed);
    canvas.on('selection:created', refresh);
    canvas.on('selection:updated', refresh);
    canvas.on('selection:cleared', refresh);
    // Guias: encaixe ao arrastar perto da margem, do centro ou de outro elemento.
    canvas.on('object:moving', ({ target, e }) => {
      // Shift: move livre, sem encaixar nas guias.
      if ((e as MouseEvent).shiftKey) { guides.current = { x: [], y: [] }; return; }
      // O Fabric avisa antes de atualizar as coordenadas do elemento.
      target.setCoords();
      const moving = new Set(canvas.getActiveObjects());
      const others = canvas.getObjects().filter(o => !moving.has(o) && o !== target && o.visible).map(o => o.getBoundingRect());
      const result = snap(target.getBoundingRect(), snapLines(width, height, others), 6 / canvas.getZoom());
      if (result.dx || result.dy) { target.set({ left: target.left + result.dx, top: target.top + result.dy }); target.setCoords(); }
      guides.current = result.guides;
    });
    const clearGuides = () => { if (guides.current.x.length || guides.current.y.length) { guides.current = { x: [], y: [] }; canvas.requestRenderAll(); } };
    canvas.on('mouse:up', clearGuides);
    // Desenhadas só na tela (a exportação desenha em outro canvas, sem elas).
    canvas.on('after:render', ({ ctx }) => {
      if (ctx !== canvas.getContext()) return;
      const { x, y } = guides.current;
      if (!x.length && !y.length && !marginsRef.current) return;
      const z = canvas.getZoom(); const v = canvas.viewportTransform;
      ctx.save();
      ctx.transform(v[0], v[1], v[2], v[3], v[4], v[5]);
      ctx.lineWidth = 1 / z;
      if (marginsRef.current) {
        const m = marginOf(width, height);
        ctx.setLineDash([6 / z, 4 / z]); ctx.strokeStyle = 'rgba(124, 92, 255, 0.55)';
        ctx.strokeRect(m, m, width - 2 * m, height - 2 * m);
        ctx.setLineDash([]);
      }
      ctx.strokeStyle = '#ff3d77';
      ctx.beginPath();
      for (const gx of x) { ctx.moveTo(gx, 0); ctx.lineTo(gx, height); }
      for (const gy of y) { ctx.moveTo(0, gy); ctx.lineTo(width, gy); }
      ctx.stroke();
      ctx.restore();
    });
    return () => { alive = false; canvasRef.current = null; void canvas.dispose(); };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Fontes do pacote: registradas e, depois, os textos são medidos de novo.
  useEffect(() => {
    let alive = true;
    void loadPackFonts().then(list => {
      if (!alive) return;
      setFonts(list);
      cache.clearFontCache();
      canvasRef.current?.getObjects().forEach(o => { if (isText(o)) { o.initDimensions(); o.setCoords(); } });
      canvasRef.current?.requestRenderAll();
    });
    return () => { alive = false; };
  }, [pack.installed]);

  // Área da página e zoom (ajustar à tela ou fixo), com Ctrl + roda.
  useEffect(() => {
    const area = host.current;
    if (!area) return;
    const measure = (w: number, h: number) => setBox(old => Math.abs(old.width - w) < 2 && Math.abs(old.height - h) < 2 ? old : { width: w, height: h });
    measure(area.clientWidth, area.clientHeight);
    // Sempre a mesma medida (com o preenchimento, como o `clientWidth`): o `contentRect` não conta o
    // preenchimento, e alternar entre as duas mudava o zoom sem parar (a página piscava e pulava).
    const observer = new ResizeObserver(() => measure(area.clientWidth, area.clientHeight));
    observer.observe(area);
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey) return;
      event.preventDefault();
      setZoom(z => Math.min(4, Math.max(0.05, (z === 'fit' ? fitRef.current : z) * (event.deltaY < 0 ? 1.1 : 1 / 1.1))));
    };
    area.addEventListener('wheel', wheel, { passive: false });
    return () => { observer.disconnect(); area.removeEventListener('wheel', wheel); };
  }, []);
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !scale) return;
    canvas.setDimensions({ width: Math.round(width * scale), height: Math.round(height * scale) });
    canvas.setZoom(scale);
    // Redimensionar apaga o bitmap do canvas: desenha já (o pedido por quadro pode não vir a tempo).
    canvas.renderAll();
  }, [scale, width, height]);

  useEffect(() => { localStorage.setItem(SIDES_KEY, JSON.stringify(sides)); }, [sides]);

  // Pincéis: com a aba Desenhar aberta, arrastar na página desenha (ou apaga, com a borracha).
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !ready) return;
    const on = panel === 'desenhar' && drawing;
    canvas.isDrawingMode = on;
    if (!on) return;
    canvas.discardActiveObject(); canvas.requestRenderAll();
    if (brush === 'borracha') {
      const eraser = new EraserBrush(canvas);
      eraser.width = brushSize;
      const off = eraser.on('end', async event => {
        event.preventDefault();
        await eraser.commit(event.detail);
        commit(); refresh();
      });
      canvas.freeDrawingBrush = eraser;
      return () => { off(); };
    }
    const tool = brush === 'spray' ? new SprayBrush(canvas) : new PencilBrush(canvas);
    tool.color = brush === 'marca-texto' ? `${brushColor}66` : brushColor;
    tool.width = brushSize;
    tool.strokeLineCap = brush === 'marca-texto' ? 'square' : 'round';
    if (tool instanceof SprayBrush) { tool.density = 24; tool.dotWidth = Math.max(1, brushSize / 10); }
    canvas.freeDrawingBrush = tool;
  }, [panel, drawing, brush, brushColor, brushSize, ready, commit, refresh]);

  const canvas = canvasRef.current;
  const active = canvas?.getActiveObject();
  const selected = canvas?.getActiveObjects() ?? [];

  function apply(props: Record<string, unknown>, target: FabricObject[] = selected) {
    for (const o of target) {
      if (isIcon(o) && ('stroke' in props || 'strokeWidth' in props)) {
        parts(o).forEach(child => child.set(props));
        o.set({ dirty: true });
      } else o.set(props);
      if (isText(o)) o.initDimensions();
      o.setCoords();
    }
    canvas?.requestRenderAll(); commit(); refresh();
  }

  function add(object: FabricObject, center = true) {
    if (!canvas) return;
    if (center) {
      const w = object.getScaledWidth(); const h = object.getScaledHeight();
      object.set({ left: (width - w) / 2, top: (height - h) / 2 });
    }
    object.setCoords();
    canvas.add(object);
    canvas.setActiveObject(object);
    canvas.requestRenderAll();
  }

  const unit = Math.min(width, height);
  function addText(kind: 'titulo' | 'subtitulo' | 'corpo') {
    const preset = { titulo: ['Adicione um título', 'Poppins', 0.09, true], subtitulo: ['Adicione um subtítulo', 'Poppins', 0.055, true], corpo: ['Adicione um pouco de texto', 'Inter', 0.035, false] }[kind];
    const [text, font, size, bold] = preset as [string, string, number, boolean];
    add(new Textbox(text, { width: width * 0.7, fontSize: Math.round(unit * size), fontFamily: fontValue(font), fontWeight: bold ? 'bold' : 'normal', fill: '#1d1d2b', textAlign: 'center' }));
  }
  function addShape(kind: 'retangulo' | 'arredondado' | 'circulo' | 'triangulo' | 'estrela' | 'linha') {
    const s = unit * 0.35; const fill = '#7c5cff';
    const star = Array.from({ length: 10 }, (_, i) => {
      const r = i % 2 ? s * 0.22 : s / 2; const a = (Math.PI / 5) * i - Math.PI / 2;
      return { x: s / 2 + r * Math.cos(a), y: s / 2 + r * Math.sin(a) };
    });
    const object = {
      retangulo: () => new Rect({ width: s * 1.4, height: s, fill }),
      arredondado: () => new Rect({ width: s * 1.4, height: s, fill, rx: s * 0.15, ry: s * 0.15 }),
      circulo: () => new Circle({ radius: s / 2, fill }),
      triangulo: () => new Triangle({ width: s, height: s, fill }),
      estrela: () => new Polygon(star, { fill }),
      linha: () => new Line([0, 0, s * 1.6, 0], { stroke: '#1d1d2b', strokeWidth: Math.max(2, Math.round(unit * 0.006)) }),
    }[kind]();
    add(object);
  }
  async function addImage(url: string, credit?: string) {
    try {
      const fitted = await fitImage(url);
      const image = await FabricImage.fromURL(fitted);
      if (credit) image.set({ credit } as Record<string, unknown>);
      const k = Math.min(1, (width * 0.6) / image.width, (height * 0.6) / image.height);
      image.scale(k);
      add(image);
    } catch (e) { setError(`Não foi possível usar a imagem: ${errorText(e)}`); }
  }
  async function addLibraryImage(item: LibraryImage) {
    setBusy('Trazendo a imagem…');
    try {
      const url = await native<string>('design_image', { file: item.file ?? null, root: item.file ? null : item.root ?? null, path: item.file ? null : item.path ?? null });
      await addImage(url);
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(null); }
  }
  async function searchOnline(query: string) {
    query = query.trim();
    if (!query) return;
    const run = ++searchRun.current;
    setError(''); setBroken(new Set());
    let sources: { id: string; label: string }[];
    try { sources = await native('design_image_sources'); }
    catch (e) { setError(errorText(e)); return; }
    if (run !== searchRun.current) return;
    setOnline({ query, banks: sources.map(b => ({ ...b, items: null })) });
    const done = (id: string, items: OnlineImage[], error?: string) => {
      if (run !== searchRun.current) return;
      setOnline(old => old && { ...old, banks: old.banks.map(b => b.id === id ? { ...b, items, error } : b) });
    };
    for (const bank of sources) {
      void native<OnlineImage[]>('design_image_search', { query, provider: bank.id }).then(items => done(bank.id, items), e => done(bank.id, [], errorText(e)));
    }
  }
  async function addOnlineImage(item: OnlineImage) {
    setBusy('Baixando a imagem…');
    try {
      const result = await native<{ url: string; credit: string }>('design_image_fetch', { item });
      await addImage(result.url, result.credit);
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(null); }
  }
  function chooseSource(source: ImageSource) { setImageSource(source); localStorage.setItem('ayla-design-imagens', source); }
  function resizeSide(side: 'left' | 'right', event: ReactPointerEvent<HTMLDivElement>) {
    event.preventDefault();
    const handle = event.currentTarget;
    try { handle.setPointerCapture(event.pointerId); } catch { /* ponteiro já solto */ }
    const startX = event.clientX; const start = sides[side];
    const move = (e: PointerEvent) => {
      const delta = side === 'left' ? e.clientX - startX : startX - e.clientX;
      setSides(old => ({ ...old, [side]: clampSidePanel(start + delta) }));
    };
    const up = () => { handle.removeEventListener('pointermove', move); handle.removeEventListener('pointerup', up); handle.removeEventListener('pointercancel', up); };
    handle.addEventListener('pointermove', move); handle.addEventListener('pointerup', up); handle.addEventListener('pointercancel', up);
  }
  function nudgeSide(side: 'left' | 'right', event: ReactKeyboardEvent) {
    const step = event.shiftKey ? 40 : 10;
    const delta = event.key === 'ArrowRight' ? step : event.key === 'ArrowLeft' ? -step : 0;
    if (!delta) return;
    event.preventDefault();
    setSides(old => ({ ...old, [side]: clampSidePanel(old[side] + (side === 'left' ? delta : -delta)) }));
  }

  function addComputerImage(file: File | undefined) {
    if (!file) return;
    if (file.size > 25 * 1024 * 1024) { setError('Imagem grande demais para o design (até 25 MB).'); return; }
    const reader = new FileReader();
    reader.onload = () => void addImage(String(reader.result));
    reader.onerror = () => setError('Não foi possível ler a imagem.');
    reader.readAsDataURL(file);
  }
  async function addIcon(svg: string) {
    try {
      const group = await iconFrom(svg, '#1d1d2b');
      group.scaleToWidth(unit * 0.25);
      add(group);
    } catch (e) { setError(`Não foi possível usar o ícone: ${errorText(e)}`); }
  }

  function setBackground(fill: Fill) {
    if (!canvas) return;
    canvas.backgroundColor = fillFrom(fill, width, height);
    canvas.requestRenderAll(); commit(); refresh();
  }

  async function restore(json: string | null) {
    if (!canvas || json === null) return;
    restoring.current = true;
    canvas.discardActiveObject();
    await canvas.loadFromJSON(json);
    restoring.current = false;
    canvas.requestRenderAll();
    setCurrent(json); refresh();
  }
  const undo = () => void restore(history.current?.undo() ?? null);
  const redo = () => void restore(history.current?.redo() ?? null);

  function remove() {
    if (!canvas || !selected.length) return;
    selected.forEach(o => canvas.remove(o));
    canvas.discardActiveObject(); canvas.requestRenderAll();
  }
  async function copy() { if (active) clipboard.current = await active.clone(EXTRA); }
  async function paste() {
    if (!canvas || !clipboard.current) return;
    const clone = await clipboard.current.clone(EXTRA);
    canvas.discardActiveObject();
    clone.set({ left: clone.left + unit * 0.03, top: clone.top + unit * 0.03, evented: true });
    if (clone instanceof ActiveSelection) {
      clone.canvas = canvas;
      clone.forEachObject(o => canvas.add(o));
      clone.setCoords();
    } else canvas.add(clone);
    clipboard.current.set({ left: clipboard.current.left + unit * 0.03, top: clipboard.current.top + unit * 0.03 });
    canvas.setActiveObject(clone);
    canvas.requestRenderAll();
  }
  async function duplicate() { await copy(); await paste(); }

  function layer(move: 'front' | 'forward' | 'backward' | 'back') {
    if (!canvas || !active) return;
    for (const o of selected) {
      if (move === 'front') canvas.bringObjectToFront(o);
      else if (move === 'forward') canvas.bringObjectForward(o);
      else if (move === 'backward') canvas.sendObjectBackwards(o);
      else canvas.sendObjectToBack(o);
    }
    canvas.requestRenderAll(); commit(); refresh();
  }
  /** Alinha a seleção na página (pelo retângulo que ela ocupa, mesmo girada). */
  function align(where: 'left' | 'center' | 'right' | 'top' | 'middle' | 'bottom') {
    if (!canvas || !active) return;
    const box = active.getBoundingRect();
    const dx = where === 'left' ? -box.left : where === 'center' ? (width - box.width) / 2 - box.left : where === 'right' ? width - box.width - box.left : 0;
    const dy = where === 'top' ? -box.top : where === 'middle' ? (height - box.height) / 2 - box.top : where === 'bottom' ? height - box.height - box.top : 0;
    active.set({ left: active.left + dx, top: active.top + dy });
    active.setCoords();
    canvas.requestRenderAll(); commit(); refresh();
  }

  function rendered(format: 'png' | 'jpeg', multiplier: number) {
    canvas!.discardActiveObject();
    canvas!.renderAll();
    const url = canvas!.toDataURL({ format, quality: 0.92, multiplier: multiplier / scale, enableRetinaScaling: false });
    refresh();
    return url;
  }
  async function save() {
    if (!canvas || !history.current) return false;
    setBusy('Salvando…'); setError('');
    try {
      const data = history.current.value;
      const preview = rendered('png', previewScale(width, height)).split(',')[1];
      const next = await native<string>('design_save', { id: id ?? null, name, width, height, data, preview });
      setId(next); setSaved(data); setSavedName(name); onSaved();
      return true;
    } catch (e) { setError(errorText(e)); return false; }
    finally { setBusy(null); }
  }
  async function exportAs(format: 'png' | 'jpg' | 'pdf', target: 'iris' | 'computer') {
    if (!canvas) return;
    setExporting(false); setBusy('Exportando…'); setError(''); setNote('');
    try {
      const multiplier = exportScale(width, height, format);
      const background = canvas.backgroundColor;
      if (format !== 'png' && !background) canvas.backgroundColor = '#ffffff';
      const url = rendered(format === 'png' ? 'png' : 'jpeg', multiplier);
      canvas.backgroundColor = background;
      const result = await native<{ cancelled?: boolean; name?: string; file?: string }>('design_export', {
        name, format, image: url.split(',')[1], pixels: [Math.round(width * multiplier), Math.round(height * multiplier)],
        page: format === 'pdf' ? pdfPage(width, height) : null, target,
      });
      if (!result.cancelled) { const message = target === 'iris' ? `${result.name} está na Iris (Arquivos).` : `Salvo como ${result.name}.`; setNote(message); onExported(message); }
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(null); canvas.requestRenderAll(); }
  }
  async function close(saveFirst: boolean) {
    if (saveFirst && !(await save())) return;
    onClose();
  }

  // Atalhos (fora de campos de texto e da edição de um texto da página).
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      const field = event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable=true]');
      const editing = isText(canvasRef.current?.getActiveObject()) && (canvasRef.current?.getActiveObject() as Textbox).isEditing;
      const mod = event.ctrlKey || event.metaKey;
      if (leave) { if (event.key === 'Escape') setLeave(null); return; }
      if (mod && event.key.toLowerCase() === 's') { event.preventDefault(); void save(); return; }
      if (field || editing) return;
      if (event.key === 'Escape') { if (canvasRef.current?.getActiveObject()) { canvasRef.current.discardActiveObject(); canvasRef.current.requestRenderAll(); refresh(); } else if (dirty) setLeave('close'); else onClose(); }
      else if (event.key === 'Delete' || event.key === 'Backspace') { event.preventDefault(); remove(); }
      else if (mod && event.key.toLowerCase() === 'z') { event.preventDefault(); if (event.shiftKey) redo(); else undo(); }
      else if (mod && event.key.toLowerCase() === 'y') { event.preventDefault(); redo(); }
      else if (mod && event.key.toLowerCase() === 'c') void copy();
      else if (mod && event.key.toLowerCase() === 'v') void paste();
      else if (mod && event.key.toLowerCase() === 'd') { event.preventDefault(); void duplicate(); }
      else if (event.key.startsWith('Arrow') && canvasRef.current?.getActiveObject()) {
        event.preventDefault();
        const o = canvasRef.current.getActiveObject()!; const step = event.shiftKey ? 10 : 1;
        o.set({ left: o.left + (event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0), top: o.top + (event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0) });
        o.setCoords(); canvasRef.current.requestRenderAll(); commit();
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });

  // Imagens da Iris (só imagens paradas), carregadas ao abrir o painel.
  useEffect(() => {
    if (panel !== 'imagens' || images) return;
    void native<{ items: (LibraryImage & { kind: string })[] }>('iris_library').then(result => setImages(result.items.filter(i => i.kind === 'image' && !/\.gif$/i.test(i.name))), e => setError(errorText(e)));
  }, [panel, images]);
  const shownImages = useMemo(() => {
    const q = imageQuery.trim().toLowerCase();
    return (images ?? []).filter(i => !q || `${i.name} ${i.place}`.toLowerCase().includes(q)).slice(0, 60);
  }, [images, imageQuery]);
  // Ícones: todos, de 60 em 60, carregando mais ao rolar até o fim (ou só os da busca).
  const loadIcons = useCallback(async (query: string, offset: number) => {
    if (loadingIcons.current && offset) return;
    loadingIcons.current = true;
    try {
      const page = await native<{ items: { name: string; svg: string }[]; more: boolean }>('design_icons', { query, offset });
      setIcons(old => offset && old?.query === query ? { query, items: [...old.items, ...page.items], more: page.more } : { query, ...page });
    } catch (e) { setError(errorText(e)); }
    finally { loadingIcons.current = false; }
  }, []);
  useEffect(() => {
    if (panel !== 'icones' || !pack.installed) return;
    const timer = setTimeout(() => void loadIcons(iconQuery.trim(), 0), 200);
    return () => clearTimeout(timer);
  }, [panel, iconQuery, pack.installed, loadIcons]);
  const iconCount = icons?.items.length ?? 0;
  const iconsMore = Boolean(icons?.more);
  const iconsQuery = icons?.query ?? '';
  useEffect(() => {
    const end = iconsEnd.current;
    if (!end || !iconsMore) return;
    const observer = new IntersectionObserver(([entry]) => { if (entry.isIntersecting) void loadIcons(iconsQuery, iconCount); }, { root: end.closest('.design-panel'), rootMargin: '0px 0px 300px 0px' });
    observer.observe(end);
    return () => observer.disconnect();
  }, [iconsMore, iconCount, iconsQuery, loadIcons]);

  const fontOptions = [...fonts.map(f => f.family), ...systemFonts];
  const text = isText(active) ? active : null;
  const backgroundFill = canvas?.backgroundColor;
  const backgroundColor = solid(backgroundFill) ?? (backgroundFill instanceof Gradient ? hex(backgroundFill.colorStops[0]?.color, '#ffffff') : '#ffffff');
  const fillValue = active && !text && !(active instanceof Line) && !isIcon(active) && !(active instanceof FabricImage) && !(active instanceof ActiveSelection) ? active.fill : undefined;

  return <div className="design-editor" role="dialog" aria-modal="true" aria-label={`Design: ${name}`}>
    <header className="design-bar">
      <button className="icon-button" data-tip="Voltar aos designs" aria-label="Voltar aos designs" onClick={() => dirty ? setLeave('close') : onClose()}><CloseIcon size={18} /></button>
      <input className="design-name" value={name} maxLength={120} aria-label="Nome do design" onChange={e => setName(e.target.value)} />
      <small className="design-size">{width} × {height} px</small>
      <span className="design-bar-gap" />
      <Tool label="Desfazer (Ctrl+Z)" onClick={undo} disabled={!history.current?.canUndo}><span aria-hidden="true">↶</span></Tool>
      <Tool label="Refazer (Ctrl+Shift+Z)" onClick={redo} disabled={!history.current?.canRedo}><span aria-hidden="true">↷</span></Tool>
      <span className="design-zoom">
        <Tool label="Diminuir zoom" onClick={() => setZoom(Math.max(0.05, scale / 1.25))}><span aria-hidden="true">−</span></Tool>
        <button className="ghost-button" title="Ajustar à tela" onClick={() => setZoom('fit')}>{Math.round(scale * 100)}%</button>
        <Tool label="Aumentar zoom" onClick={() => setZoom(Math.min(4, scale * 1.25))}><span aria-hidden="true">+</span></Tool>
      </span>
      {busy && <span className="design-status" role="status">{busy}</span>}
      {!busy && ready && <span className="design-status">{note || (dirty ? 'Alterações não salvas' : id ? 'Salvo' : '')}</span>}
      <button className="ghost-button" disabled={!ready || Boolean(busy)} onClick={() => void save()}>Salvar</button>
      <span className="design-export">
        <button className="primary-action" disabled={!ready || Boolean(busy)} aria-haspopup="menu" aria-expanded={exporting} onClick={() => setExporting(v => !v)}><DownloadIcon size={15} /> Exportar</button>
        {exporting && <div className="design-menu" role="menu">
          {(['png', 'jpg', 'pdf'] as const).map(format => <div key={format} className="design-menu-row">
            <b>{format.toUpperCase()}</b>
            <button role="menuitem" className="ghost-button" onClick={() => void exportAs(format, 'iris')}>Para a Iris</button>
            <button role="menuitem" className="ghost-button" onClick={() => void exportAs(format, 'computer')}>No computador…</button>
          </div>)}
          <small>{`PNG e JPG com ${Math.round(width * exportScale(width, height, 'png'))} px de largura; PDF em alta resolução.`}</small>
        </div>}
      </span>
    </header>
    {error && <p role="alert" className="app-error design-error">{error} <button className="link-button" onClick={() => setError('')}>Fechar</button></p>}
    {leave && <div className="dialog-backdrop design-leave" onPointerDown={e => { if (e.target === e.currentTarget) setLeave(null); }}>
      <div className="dialog" role="alertdialog" aria-modal="true" aria-labelledby="design-leave-title" aria-describedby="design-leave-text">
        <h2 id="design-leave-title">Sair sem salvar?</h2>
        <p id="design-leave-text">“{name}” tem alterações que ainda não foram salvas. Se sair sem salvar, elas se perdem.</p>
        <div className="design-leave-actions">
          <button className="ghost-button" autoFocus onClick={() => setLeave(null)}>Continuar editando</button>
          <button className="ghost-button danger" onClick={() => void close(false)}>Sair sem salvar</button>
          <button className="primary-action" onClick={() => void close(true)}>Salvar e sair</button>
        </div>
      </div>
    </div>}
    <div className="design-body">
      <nav className="design-tabs" aria-label="Elementos">
        {([['texto', 'Texto', 'T'], ['formas', 'Formas', '◆'], ['imagens', 'Imagens', null], ['icones', 'Ícones', '★'], ['desenhar', 'Desenhar', '✎'], ['fundo', 'Fundo', '▦']] as const).map(([key, label, glyph]) =>
          <button key={key} className={panel === key ? 'design-tab active' : 'design-tab'} aria-pressed={panel === key} onClick={() => setPanel(key)}>
            <span aria-hidden="true">{glyph ?? <ImageIcon size={18} />}</span>{label}
          </button>)}
      </nav>
      <aside className="design-panel" aria-label="Adicionar" style={{ width: sides.left }}>
        {panel === 'texto' && <div className="design-stack">
          <button className="design-add title" onClick={() => addText('titulo')}>Adicionar um título</button>
          <button className="design-add subtitle" onClick={() => addText('subtitulo')}>Adicionar um subtítulo</button>
          <button className="design-add body" onClick={() => addText('corpo')}>Adicionar um texto</button>
          <p className="design-hint">Clique duas vezes num texto da página para escrever.</p>
          {!pack.installed && <PackOffer pack={pack} what="12 fontes livres e mais de 2.000 ícones" />}
        </div>}
        {panel === 'formas' && <div className="design-shapes">
          {([['retangulo', 'Retângulo', <rect key="r" x="3" y="6" width="18" height="12" />], ['arredondado', 'Arredondado', <rect key="a" x="3" y="6" width="18" height="12" rx="4" />],
            ['circulo', 'Círculo', <circle key="c" cx="12" cy="12" r="8" />], ['triangulo', 'Triângulo', <path key="t" d="M12 4 21 20H3z" />],
            ['estrela', 'Estrela', <path key="e" d="m12 3 2.6 5.6 6.1.7-4.5 4.2 1.2 6L12 16.6 6.6 19.5l1.2-6-4.5-4.2 6.1-.7z" />], ['linha', 'Linha', <path key="l" d="M3 12h18" fill="none" strokeWidth="2.5" />]] as const)
            .map(([kind, label, shape]) => <button key={kind} className="design-shape" title={label} aria-label={label} onClick={() => addShape(kind)}>
              <svg viewBox="0 0 24 24" width="42" height="42" fill="currentColor" stroke="currentColor" aria-hidden="true">{shape}</svg><small>{label}</small>
            </button>)}
        </div>}
        {panel === 'imagens' && <div className="design-stack">
          <div className="design-sources" role="tablist" aria-label="De onde vem a imagem">
            {([['iris', 'Iris'], ['computador', 'Sistema'], ['online', 'Bancos']] as const).map(([key, label]) =>
              <button key={key} role="tab" aria-selected={imageSource === key} className={imageSource === key ? 'active' : ''} onClick={() => chooseSource(key)}>{label}</button>)}
          </div>
          {imageSource === 'iris' && <>
            <input className="organ-search" type="search" value={imageQuery} placeholder="Buscar nas imagens da Iris" aria-label="Buscar nas imagens da Iris" onChange={e => setImageQuery(e.target.value)} />
            {images === null ? <p className="design-hint" role="status">Reunindo as imagens…</p>
              : !shownImages.length ? <p className="design-hint">Nenhuma imagem encontrada.</p>
              : <div className="design-thumbs">{shownImages.map(item => <button key={item.key} title={`${item.name}\n${item.place}`} disabled={Boolean(busy)} onClick={() => void addLibraryImage(item)}>
                  <img src={mediaUrl((item.file ? { file: item.file } : { root: item.root ?? '', path: item.path ?? '' }) as MediaRef, true)} alt={item.name} loading="lazy" />
                </button>)}</div>}
          </>}
          {imageSource === 'computador' && <>
            <label className="design-drop">
              <ImageIcon size={26} />
              <b>Escolher uma imagem do computador</b>
              <small>PNG, JPG, WebP, SVG, AVIF ou BMP, até 25 MB. A imagem fica guardada dentro do design.</small>
              <input type="file" accept="image/png,image/jpeg,image/webp,image/svg+xml,image/avif,image/bmp" hidden onChange={e => { addComputerImage(e.target.files?.[0]); e.target.value = ''; }} />
            </label>
          </>}
          {imageSource === 'online' && <>
            <form className="design-online-search" onSubmit={e => { e.preventDefault(); void searchOnline(onlineQuery); }}>
              <input className="organ-search" type="search" value={onlineQuery} placeholder="Buscar fotos (em inglês acha mais)" aria-label="Buscar nos bancos de imagens" onChange={e => setOnlineQuery(e.target.value)} />
              <button className="ghost-button" type="submit" disabled={!onlineQuery.trim()}>Buscar</button>
            </form>
            {!online ? <p className="design-hint">Só o termo buscado é enviado, e só quando você busca. Openverse sempre; Unsplash, Pexels e Pixabay quando tiverem a chave em Configurações › Busca de imagens. Só aparecem imagens livres para usar e modificar.</p>
              : (() => {
                  const pending = online.banks.filter(b => b.items === null).map(b => b.label);
                  const failed = online.banks.filter(b => b.error).map(b => b.label);
                  const items = online.banks.flatMap(b => b.items ?? []).filter(item => !broken.has(`${item.provider}-${item.id}`));
                  return <>
                    <p className="design-hint" role="status">
                      {pending.length ? `Buscando em ${pending.join(', ')}… ` : !items.length ? 'Nenhuma imagem encontrada. ' : ''}
                      {failed.length ? `${failed.join(' e ')} não ${failed.length > 1 ? 'responderam' : 'respondeu'} agora.` : ''}
                    </p>
                    <div className="design-thumbs">{items.map(item => {
                      const key = `${item.provider}-${item.id}`;
                      return <button key={key} disabled={Boolean(busy)} title={`${item.author || 'Autor desconhecido'} · ${item.license_name}`} onClick={() => void addOnlineImage(item)}>
                        <img src={item.thumb} alt={item.author ? `Foto de ${item.author}` : ''} loading="lazy" referrerPolicy="no-referrer"
                          onError={() => setBroken(old => new Set(old).add(key))} />
                      </button>;
                    })}</div>
                  </>;
                })()}
          </>}
        </div>}
        {panel === 'icones' && <div className="design-stack">
          {!pack.installed ? <PackOffer pack={pack} what="mais de 2.000 ícones (Lucide) e 12 fontes livres" />
            : <>
                <input className="organ-search" type="search" value={iconQuery} placeholder="Buscar ícones (em inglês: heart, star…)" aria-label="Buscar ícones" onChange={e => setIconQuery(e.target.value)} />
                {icons === null ? <p className="design-hint" role="status">Carregando…</p> : !icons.items.length ? <p className="design-hint">Nenhum ícone com esse nome.</p>
                  : <>
                      <div className="design-icons">{icons.items.map(icon => <button key={icon.name} title={icon.name} aria-label={icon.name} onClick={() => void addIcon(icon.svg)}>
                        <img src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(icon.svg)}`} alt="" loading="lazy" />
                      </button>)}</div>
                      {icons.more && <button ref={iconsEnd} className="ghost-button" onClick={() => void loadIcons(icons.query, icons.items.length)}>Mais ícones</button>}
                    </>}
              </>}
        </div>}
        {panel === 'desenhar' && <div className="design-stack">
          <div className="design-brushes" role="radiogroup" aria-label="Pincel">
            {brushes.map(([key, label]) => <button key={key} role="radio" aria-checked={drawing && brush === key} className={drawing && brush === key ? 'active' : ''} onClick={() => { setBrush(key); setDrawing(true); }}>
              <BrushGlyph kind={key} /><small>{label}</small>
            </button>)}
          </div>
          <button className={drawing ? 'ghost-button' : 'ghost-button active'} aria-pressed={!drawing} onClick={() => setDrawing(d => !d)}>{drawing ? 'Parar de desenhar (selecionar)' : 'Voltar a desenhar'}</button>
          {brush !== 'borracha' && <>
            <label className="design-field">Cor <input type="color" value={brushColor} onChange={e => setBrushColor(e.target.value)} /></label>
            <div className="design-swatches">{swatches.map(c => <button key={c} style={{ background: c }} aria-label={`Pincel ${c}`} onClick={() => setBrushColor(c)} />)}</div>
          </>}
          <label className="design-field">Espessura: {brushSize} px <input type="range" min={1} max={Math.max(60, Math.round(unit * 0.12))} value={brushSize} onChange={e => { const v = Number(e.target.value); setBrushSizes(old => ({ ...old, [brush]: v })); }} /></label>
          <p className="design-hint">{brush === 'borracha' ? 'Arraste sobre traços, formas ou imagens para apagar partes deles. Ctrl+Z desfaz.' : 'Arraste na página para desenhar. Cada traço vira um elemento: dá para mover, mudar a transparência ou apagar.'}</p>
        </div>}
        {panel === 'fundo' && <div className="design-stack">
          <label className="design-field">Cor do fundo <input type="color" value={backgroundColor} onChange={e => setBackground(e.target.value)} /></label>
          <div className="design-swatches">{swatches.map(c => <button key={c} style={{ background: c }} aria-label={`Fundo ${c}`} onClick={() => setBackground(c)} />)}</div>
          <p className="sidebar-label">Gradientes</p>
          <div className="design-swatches big">{gradients.map((g, i) => <button key={i} style={{ background: fillCss(g) }} aria-label={`Gradiente ${i + 1}`} onClick={() => setBackground(g)} />)}</div>
          <button className="ghost-button" onClick={() => { if (canvas) { canvas.backgroundColor = ''; canvas.requestRenderAll(); commit(); refresh(); } }}>Fundo transparente (PNG)</button>
        </div>}
      </aside>
      <div className="design-resizer" role="separator" aria-orientation="vertical" aria-label="Largura do painel da esquerda" aria-valuemin={sideLimits.min} aria-valuemax={sideLimits.max} aria-valuenow={sides.left} tabIndex={0}
        onPointerDown={e => resizeSide('left', e)} onKeyDown={e => nudgeSide('left', e)} onDoubleClick={() => setSides(old => ({ ...old, left: 264 }))} />
      <div className={zoom === 'fit' ? 'design-stage fit' : 'design-stage'} ref={host} onPointerDown={e => { if (e.target === e.currentTarget && canvas) { canvas.discardActiveObject(); canvas.requestRenderAll(); refresh(); } }}>
        <div className="design-page" style={{ width: Math.round(width * scale), height: Math.round(height * scale) }}><canvas ref={element} /></div>
      </div>
      <div className="design-resizer" role="separator" aria-orientation="vertical" aria-label="Largura do painel da direita" aria-valuemin={sideLimits.min} aria-valuemax={sideLimits.max} aria-valuenow={sides.right} tabIndex={0}
        onPointerDown={e => resizeSide('right', e)} onKeyDown={e => nudgeSide('right', e)} onDoubleClick={() => setSides(old => ({ ...old, right: 264 }))} />
      <aside className="design-props" aria-label="Propriedades" style={{ width: sides.right }}>
        {!active ? <div className="design-stack">
          <p className="sidebar-label">Página</p>
          <p className="design-hint">{width} × {height} px. Clique num elemento para editar; arraste para mover; use as alças para girar e redimensionar.</p>
          <label className="design-field">Fundo <input type="color" value={backgroundColor} onChange={e => setBackground(e.target.value)} /></label>
          <label className="design-check"><input type="checkbox" checked={margins} onChange={e => { setMargins(e.target.checked); localStorage.setItem('ayla-design-margens', e.target.checked ? 'sim' : 'nao'); canvas?.requestRenderAll(); }} /> Mostrar margens</label>
          <p className="design-hint">Ao arrastar, os elementos encaixam nas margens, no centro da página e nas bordas dos outros; a linha rosa mostra o alinhamento. As margens não aparecem na exportação.</p>
          <p className="design-hint">Atalhos: Ctrl+Z desfaz, Ctrl+D duplica, Delete apaga, setas movem (Shift: 10 px), Ctrl+S salva.</p>
        </div> : <div className="design-stack">
          {text && <>
            <p className="sidebar-label">Texto</p>
            <label className="design-field">Fonte <select value={fontName(text.fontFamily)} onChange={e => apply({ fontFamily: fontValue(e.target.value) })}>
              {!fontOptions.includes(fontName(text.fontFamily)) && <option value={fontName(text.fontFamily)}>{fontName(text.fontFamily)} (sem o pacote)</option>}
              {fontOptions.map(f => <option key={f} value={f} style={{ fontFamily: fontValue(f) }}>{f}</option>)}
            </select></label>
            <div className="design-row">
              <label className="design-field">Tamanho <input type="number" min={6} max={2000} value={Math.round(text.fontSize * (text.scaleY ?? 1))} onChange={e => apply({ fontSize: Math.max(6, Number(e.target.value) || 6) / (text.scaleY || 1) })} /></label>
              <label className="design-field">Cor <input type="color" value={hex(text.fill)} onChange={e => apply({ fill: e.target.value })} /></label>
            </div>
            <div className="design-toggles" role="group" aria-label="Estilo">
              <Tool label="Negrito" active={text.fontWeight === 'bold'} onClick={() => apply({ fontWeight: text.fontWeight === 'bold' ? 'normal' : 'bold' })}><b>N</b></Tool>
              <Tool label="Itálico" active={text.fontStyle === 'italic'} onClick={() => apply({ fontStyle: text.fontStyle === 'italic' ? 'normal' : 'italic' })}><i>I</i></Tool>
              <Tool label="Sublinhado" active={Boolean(text.underline)} onClick={() => apply({ underline: !text.underline })}><u>S</u></Tool>
              {(['left', 'center', 'right', 'justify'] as const).map(a => <Tool key={a} label={{ left: 'Alinhar à esquerda', center: 'Centralizar', right: 'Alinhar à direita', justify: 'Justificar' }[a]} active={text.textAlign === a} onClick={() => apply({ textAlign: a })}>
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true"><path d={{ left: 'M4 6h16M4 10h10M4 14h16M4 18h10', center: 'M4 6h16M7 10h10M4 14h16M7 18h10', right: 'M4 6h16M10 10h10M4 14h16M10 18h10', justify: 'M4 6h16M4 10h16M4 14h16M4 18h16' }[a]} /></svg>
              </Tool>)}
            </div>
            <label className="design-field">Espaço entre letras <input type="range" min={-100} max={800} step={10} value={text.charSpacing} onChange={e => apply({ charSpacing: Number(e.target.value) })} /></label>
            <label className="design-field">Altura da linha <input type="range" min={0.7} max={2.5} step={0.05} value={text.lineHeight} onChange={e => apply({ lineHeight: Number(e.target.value) })} /></label>
          </>}
          {fillValue !== undefined && <>
            <p className="sidebar-label">Forma</p>
            <label className="design-field">Cor {fillValue instanceof Gradient && <small>(gradiente)</small>}<input type="color" value={fillValue instanceof Gradient ? hex(fillValue.colorStops[0]?.color) : hex(fillValue)} onChange={e => apply({ fill: e.target.value })} /></label>
            <div className="design-swatches">{swatches.map(c => <button key={c} style={{ background: c }} aria-label={`Cor ${c}`} onClick={() => apply({ fill: c })} />)}</div>
            <button className="ghost-button" onClick={() => {
              const [a, b] = fillValue instanceof Gradient ? [hex(fillValue.colorStops[0]?.color), hex(fillValue.colorStops[1]?.color)] : [hex(fillValue), '#ffffff'];
              apply({ fill: new Gradient({ type: 'linear', gradientUnits: 'pixels', coords: gradientCoords(90, active!.width, active!.height), colorStops: [{ offset: 0, color: a }, { offset: 1, color: b }] }) });
            }}>Usar gradiente</button>
            {fillValue instanceof Gradient && <label className="design-field">Cor final <input type="color" value={hex(fillValue.colorStops[1]?.color)} onChange={e => {
              const stops = fillValue.colorStops.map((s, i) => i === fillValue.colorStops.length - 1 ? { ...s, color: e.target.value } : s);
              apply({ fill: new Gradient({ type: 'linear', gradientUnits: 'pixels', coords: fillValue.coords, colorStops: stops }) });
            }} /></label>}
            {active instanceof Rect && <label className="design-field">Cantos arredondados <input type="range" min={0} max={Math.round(Math.min(active.width, active.height) / 2)} value={active.rx ?? 0} onChange={e => apply({ rx: Number(e.target.value), ry: Number(e.target.value) })} /></label>}
            <div className="design-row">
              <label className="design-field">Contorno <input type="color" value={hex(active.stroke, '#000000')} onChange={e => apply({ stroke: e.target.value, strokeWidth: active.strokeWidth || Math.max(2, Math.round(unit * 0.004)) })} /></label>
              <label className="design-field">Espessura <input type="number" min={0} max={200} value={active.stroke ? active.strokeWidth : 0} onChange={e => apply({ strokeWidth: Math.max(0, Number(e.target.value) || 0), stroke: active.stroke || '#1d1d2b' })} /></label>
            </div>
          </>}
          {(active instanceof Line || isIcon(active)) && <>
            <p className="sidebar-label">{isIcon(active) ? 'Ícone' : 'Linha'}</p>
            <div className="design-row">
              <label className="design-field">Cor <input type="color" value={hex(isIcon(active) ? parts(active)[0]?.stroke : active.stroke)} onChange={e => apply({ stroke: e.target.value })} /></label>
              <label className="design-field">Espessura <input type="number" min={0.5} max={200} step={0.5} value={(isIcon(active) ? parts(active)[0]?.strokeWidth : active.strokeWidth) ?? 1} onChange={e => apply({ strokeWidth: Math.max(0.5, Number(e.target.value) || 1) })} /></label>
            </div>
          </>}
          {typeof (active as FabricObject & { credit?: string }).credit === 'string' && <>
            <p className="sidebar-label">Crédito</p>
            <p className="design-hint design-credit">{(active as FabricObject & { credit?: string }).credit}</p>
            <button className="ghost-button" onClick={() => void navigator.clipboard?.writeText((active as FabricObject & { credit?: string }).credit ?? '')}><CopyIcon size={15} /> Copiar crédito</button>
          </>}
          <label className="design-field">Transparência <input type="range" min={0.05} max={1} step={0.05} value={active.opacity} onChange={e => apply({ opacity: Number(e.target.value) })} /></label>
          <p className="sidebar-label">Camada</p>
          <div className="design-grid-buttons">
            <button className="ghost-button" onClick={() => layer('front')}>Para a frente</button>
            <button className="ghost-button" onClick={() => layer('forward')}>Avançar</button>
            <button className="ghost-button" onClick={() => layer('backward')}>Recuar</button>
            <button className="ghost-button" onClick={() => layer('back')}>Para trás</button>
          </div>
          <p className="sidebar-label">Alinhar na página</p>
          <div className="design-grid-buttons three">
            {([['left', 'Esquerda'], ['center', 'Centro'], ['right', 'Direita'], ['top', 'Topo'], ['middle', 'Meio'], ['bottom', 'Base']] as const).map(([where, label]) => <button key={where} className="ghost-button" onClick={() => align(where)}>{label}</button>)}
          </div>
          <div className="design-row">
            <button className="ghost-button" onClick={() => void duplicate()}><CopyIcon size={15} /> Duplicar</button>
            <button className="ghost-button" onClick={remove}><TrashIcon size={15} /> Apagar</button>
          </div>
        </div>}
      </aside>
    </div>
  </div>;
}

function BrushGlyph({ kind }: { kind: Brush }) {
  const d = { caneta: 'M4 20c3-1 5-5 8-9s5-6 8-7', marcador: 'M4 18c4-2 6-8 10-10s5-2 6-2', 'marca-texto': 'M3 15h18', spray: '', borracha: 'M7 20h10M5 15l8-9 6 6-8 9H8z' }[kind];
  return <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"
    strokeWidth={{ caneta: 1.6, marcador: 3.2, 'marca-texto': 7, spray: 1, borracha: 1.8 }[kind]} opacity={kind === 'marca-texto' ? 0.5 : 1}>
    {kind === 'spray' ? [[6, 8], [9, 5], [12, 9], [15, 6], [18, 10], [8, 13], [12, 14], [16, 15], [10, 18], [14, 19]].map(([x, y]) => <circle key={`${x}-${y}`} cx={x} cy={y} r="1.1" fill="currentColor" />) : <path d={d} />}
  </svg>;
}

/** Convite para baixar o pacote de recursos (fontes e ícones), com progresso. */
export function PackOffer({ pack, what }: { pack: Pack; what: string }) {
  const mb = (pack.bytes / 1024 ** 2).toFixed(0);
  return <div className="design-pack">
    <p><b>Pacote de recursos (opcional)</b><br />{what}, de licença livre. Download de {mb} MB, feito uma vez.</p>
    {pack.progress ? <p role="status">{pack.progress}</p> : <button className="primary-action" onClick={pack.install}>Baixar pacote</button>}
  </div>;
}

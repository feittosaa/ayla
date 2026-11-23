import { useEffect, useRef, useState, type CSSProperties, type PointerEvent } from 'react';
import { RefreshIcon } from './Icons';
import { NoteGraphSimulation, type Point } from '../services/NoteGraphLayout';

export type NoteGraph = {
  nodes: { root: string; rootName: string; path: string; title: string }[];
  edges: [number, number][];
};

/** Câmera em coordenadas do mapa: `x`/`y` é o ponto no centro da tela. */
type Camera = Point & { zoom: number };
type Drag = { start: Point; camera: Camera; moved: boolean; node?: number };
type Colors = { accent: string; node: string; edge: string; text: string; background: string };

const heightKey = 'ayla-athena-graph-height';
const keyOf = (note: { root: string; path: string }) => `${note.root}|${note.path}`;
const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));
/** Posições por nota entre trocas de pasta e atualizações, enquanto a Athena estiver aberta. */
const remembered = new Map<string, Point>();

function readHeight() {
  const saved = Number(localStorage.getItem(heightKey));
  return Number.isFinite(saved) && saved >= 140 ? saved : undefined;
}
const reducedMotion = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;
const radiusOf = (degree: number) => Math.min(8, 2.3 + Math.sqrt(degree) * 1.1);
/** Tamanho na tela proporcional ao zoom (de longe os pontos encolhem e não se fundem), limitado de perto. */
const screenRadius = (degree: number, zoom: number) => Math.max(0.8, radiusOf(degree) * Math.min(zoom, 3));

/**
 * Graph View da Athena no estilo do Obsidian: simulação de forças que assenta aos poucos, pontos do tamanho
 * das conexões, vizinhança destacada ao passar o mouse e nomes que aparecem conforme o zoom.
 */
export default function AthenaGraph({ graph, activeNote, onOpen, onRefresh, error, scope }: {
  graph: NoteGraph | null; activeNote?: { root: string; path: string }; scope?: string;
  onOpen: (root: string, path: string) => void; onRefresh: () => void; error: string;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const section = useRef<HTMLElement>(null);
  const heightDrag = useRef<{ y: number; start: number; current: number; maximum: number } | null>(null);
  const [height, setHeight] = useState(readHeight);
  const [hover, setHover] = useState<number>();
  const [keyboard, setKeyboard] = useState<number>();
  const sim = useRef<{ sim: NoteGraphSimulation; keys: string[]; graph: NoteGraph } | null>(null);
  const camera = useRef<Camera>({ x: 0, y: 0, zoom: 1 });
  /** Enquanto ninguém mexe na câmera, ela acompanha o mapa inteiro. */
  const following = useRef(true);
  const fade = useRef(0);
  const drag = useRef<Drag | null>(null);
  const size = useRef({ width: 1, height: 1 });
  const colors = useRef<Colors | null>(null);
  const loop = useRef(0);
  const view = useRef({ hover: undefined as number | undefined, active: -1, keyboard: undefined as number | undefined });

  const hasNodes = Boolean(graph?.nodes.length);
  const active = graph?.nodes.findIndex(note => note.root === activeNote?.root && note.path === activeNote?.path) ?? -1;
  view.current = { hover, active, keyboard };

  /** Reaproveita as posições conhecidas quando as notas ou os links mudam. */
  if (graph && sim.current?.graph !== graph) {
    const previous = sim.current;
    if (previous) for (const [key, point] of previous.sim.positions(previous.keys)) remembered.set(key, point);
    const keys = graph.nodes.map(keyOf);
    const next = new NoteGraphSimulation(keys, graph.edges, remembered);
    if (reducedMotion()) { for (let i = 0; i < 400 && next.running; i++) next.tick(); }
    else if (next.alpha > 0.5) {
      const until = performance.now() + 10;
      while (next.running && performance.now() < until) next.tick();
    }
    sim.current = { sim: next, keys, graph };
    if (!previous || previous.graph.nodes[0]?.root !== graph.nodes[0]?.root) following.current = true;
  }

  const toScreen = (x: number, y: number) => {
    const { zoom } = camera.current;
    return { x: (x - camera.current.x) * zoom + size.current.width / 2, y: (y - camera.current.y) * zoom + size.current.height / 2 };
  };
  const toWorld = (x: number, y: number) => {
    const { zoom } = camera.current;
    return { x: (x - size.current.width / 2) / zoom + camera.current.x, y: (y - size.current.height / 2) / zoom + camera.current.y };
  };

  function fitTarget(): Camera {
    const current = sim.current?.sim;
    if (!current?.count) return { x: 0, y: 0, zoom: 1 };
    const { minX, minY, maxX, maxY } = current.bounds();
    const pad = 36;
    const zoom = Math.min((size.current.width - pad) / Math.max(1, maxX - minX), (size.current.height - pad) / Math.max(1, maxY - minY), 1.2);
    return { x: (minX + maxX) / 2, y: (minY + maxY) / 2, zoom: Math.max(0.05, zoom) };
  }

  function draw() {
    const element = canvas.current; const state = sim.current;
    if (!element || !state) return;
    const { sim: graphSim, graph: data } = state;
    const theme = colors.current ??= (() => {
      const style = getComputedStyle(element);
      const read = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
      return {
        accent: read('--ayla-accent', '#e99449'), node: read('--ayla-text-soft', '#999'),
        edge: read('--ayla-text-faint', '#777'), text: read('--ayla-text', '#eee'), background: read('--ayla-sidebar', '#1e1e1e'),
      };
    })();
    const { width, height } = size.current;
    const density = Math.min(window.devicePixelRatio || 1, 2);
    const pixelWidth = Math.round(width * density); const pixelHeight = Math.round(height * density);
    if (element.width !== pixelWidth || element.height !== pixelHeight) { element.width = pixelWidth; element.height = pixelHeight; }
    const context = element.getContext('2d');
    if (!context) return;
    context.setTransform(density, 0, 0, density, 0, 0);
    context.clearRect(0, 0, width, height);

    const { hover: hovered, active: current, keyboard: chosen } = view.current;
    const wanted = hovered ?? chosen;
    const focus = wanted !== undefined && wanted < graphSim.count ? wanted : undefined;
    const near = new Set<number>();
    if (focus !== undefined) { near.add(focus); for (const n of graphSim.neighbors[focus] ?? []) near.add(n); }
    const dim = fade.current;
    const { zoom } = camera.current;
    const screen: Point[] = [];
    for (let i = 0; i < graphSim.count; i++) screen.push(toScreen(graphSim.x[i], graphSim.y[i]));
    const visible = (p: Point, margin: number) => p.x > -margin && p.x < width + margin && p.y > -margin && p.y < height + margin;

    context.lineWidth = clamp(zoom, 0.25, 1.4);
    context.strokeStyle = theme.edge;
    context.globalAlpha = 0.42 * (1 - 0.7 * dim);
    context.beginPath();
    for (const [a, b] of data.edges) {
      if (focus !== undefined && (a === focus || b === focus)) continue;
      context.moveTo(screen[a].x, screen[a].y); context.lineTo(screen[b].x, screen[b].y);
    }
    context.stroke();
    if (focus !== undefined) {
      context.globalAlpha = 0.35 + 0.55 * dim;
      context.strokeStyle = theme.accent;
      context.lineWidth = clamp(zoom * 1.4, 0.6, 2);
      context.beginPath();
      for (const n of graphSim.neighbors[focus] ?? []) { context.moveTo(screen[focus].x, screen[focus].y); context.lineTo(screen[n].x, screen[n].y); }
      context.stroke();
    }

    /** Nomes discretos só com zoom alto (aparecem aos poucos entre 1,25× e 2×). */
    const closeUp = clamp((zoom - 1.25) / 0.75, 0, 1) * 0.6;
    const labels: { at: Point; text: string; alpha: number; strong: boolean }[] = [];
    for (let i = 0; i < graphSim.count; i++) {
      const point = screen[i];
      const radius = screenRadius(graphSim.degree(i), zoom);
      if (!visible(point, radius + 60)) continue;
      const highlighted = near.has(i);
      const lit = i === focus || i === current;
      context.globalAlpha = highlighted || i === current ? 1 : 1 - 0.75 * dim;
      context.fillStyle = lit ? theme.accent : theme.node;
      context.beginPath();
      context.arc(point.x, point.y, radius, 0, Math.PI * 2);
      context.fill();
      if (i === current && i !== focus) {
        context.globalAlpha = 0.5; context.lineWidth = 1.5; context.strokeStyle = theme.accent;
        context.beginPath(); context.arc(point.x, point.y, radius + 3 * Math.min(1, zoom), 0, Math.PI * 2); context.stroke();
      }
      // Em destaque só a nota sob o mouse (ou escolhida pelo teclado); de perto, as outras com nome discreto.
      const at = { x: point.x, y: point.y + radius + 11 };
      if (i === focus) labels.push({ at, text: data.nodes[i].title, alpha: Math.max(dim, 0.4), strong: true });
      else if (closeUp > 0.02) labels.push({ at, text: data.nodes[i].title, alpha: closeUp * (1 - 0.8 * dim), strong: false });
    }
    context.textAlign = 'center';
    context.textBaseline = 'middle';
    context.lineJoin = 'round';
    labels.sort((a, b) => Number(a.strong) - Number(b.strong));
    for (const label of labels) {
      context.font = label.strong ? '600 11px system-ui, sans-serif' : '400 10px system-ui, sans-serif';
      const text = label.text.length > 42 ? `${label.text.slice(0, 40)}…` : label.text;
      context.globalAlpha = label.alpha;
      context.lineWidth = 3; context.strokeStyle = theme.background;
      context.strokeText(text, label.at.x, label.at.y);
      context.fillStyle = theme.text;
      context.fillText(text, label.at.x, label.at.y);
    }
    context.globalAlpha = 1;
  }

  /** Um quadro: avança as forças, acompanha a câmera, suaviza o destaque e desenha. */
  function frame() {
    loop.current = 0;
    const state = sim.current;
    if (!state) return;
    let busy = false;
    if (state.sim.running) {
      const until = performance.now() + 6;
      do state.sim.tick(); while (state.sim.count < 400 && state.sim.running && performance.now() < until && state.sim.alpha > 0.3);
      busy = true;
    }
    if (following.current && !drag.current) {
      const target = fitTarget();
      const ease = reducedMotion() ? 1 : 0.18;
      const now = camera.current;
      const next = { x: now.x + (target.x - now.x) * ease, y: now.y + (target.y - now.y) * ease, zoom: now.zoom * (target.zoom / now.zoom) ** ease };
      camera.current = next;
      if (Math.abs(next.zoom / target.zoom - 1) > 0.002 || Math.hypot(next.x - target.x, next.y - target.y) * next.zoom > 0.5) busy = true;
    }
    const goal = view.current.hover !== undefined || view.current.keyboard !== undefined ? 1 : 0;
    if (fade.current !== goal) {
      fade.current = reducedMotion() ? goal : clamp(fade.current + (goal ? 0.12 : -0.12), 0, 1);
      busy = true;
    }
    draw();
    if (busy) loop.current = requestAnimationFrame(frame);
  }
  const frameRef = useRef(frame);
  frameRef.current = frame;
  const kick = () => { if (!loop.current) loop.current = requestAnimationFrame(() => frameRef.current()); };

  useEffect(() => () => {
    cancelAnimationFrame(loop.current);
    const state = sim.current;
    if (state) for (const [key, point] of state.sim.positions(state.keys)) remembered.set(key, point);
  }, []);
  // Índices de destaque não valem para outro conjunto de notas.
  useEffect(() => { setHover(undefined); setKeyboard(undefined); }, [graph]);
  // Redesenha quando os dados, a nota aberta ou o destaque mudam.
  useEffect(kick, [graph, active, hover, keyboard]);

  useEffect(() => {
    const element = canvas.current;
    if (!element || !hasNodes) return;
    const resize = new ResizeObserver(([entry]) => {
      size.current = { width: Math.max(1, entry.contentRect.width), height: Math.max(1, entry.contentRect.height) };
      colors.current = null;
      kick();
    });
    resize.observe(element);
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const rect = element.getBoundingClientRect();
      const anchor = toWorld(event.clientX - rect.left, event.clientY - rect.top);
      const before = camera.current;
      const zoom = clamp(before.zoom * Math.exp(-event.deltaY * 0.0018), 0.04, 8);
      const ratio = before.zoom / zoom;
      following.current = false;
      camera.current = { zoom, x: anchor.x - (anchor.x - before.x) * ratio, y: anchor.y - (anchor.y - before.y) * ratio };
      kick();
    };
    element.addEventListener('wheel', wheel, { passive: false });
    // O tema pode mudar com a Athena aberta.
    const theme = new MutationObserver(() => { colors.current = null; kick(); });
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style', 'data-theme'] });
    return () => { resize.disconnect(); element.removeEventListener('wheel', wheel); theme.disconnect(); };
  }, [hasNodes]);

  function hit(event: PointerEvent<HTMLCanvasElement>) {
    const state = sim.current;
    if (!state) return undefined;
    const rect = event.currentTarget.getBoundingClientRect();
    const point = toWorld(event.clientX - rect.left, event.clientY - rect.top);
    const { zoom } = camera.current;
    let closest: number | undefined;
    let best = Infinity;
    for (let i = 0; i < state.sim.count; i++) {
      const reach = Math.max(screenRadius(state.sim.degree(i), zoom), 7) / zoom;
      const distance = Math.hypot(state.sim.x[i] - point.x, state.sim.y[i] - point.y);
      if (distance <= reach && distance < best) { closest = i; best = distance; }
    }
    return closest;
  }

  const open = (index?: number) => {
    const note = index === undefined ? undefined : graph?.nodes[index];
    if (note) onOpen(note.root, note.path);
  };

  function release() {
    const start = drag.current;
    drag.current = null;
    if (start?.node !== undefined && start.moved && sim.current) {
      sim.current.sim.release(start.node);
      sim.current.sim.heat(0);
      kick();
    }
    return start;
  }

  const recenter = () => { following.current = true; setKeyboard(undefined); kick(); };
  const heightLimit = () => {
    const panel = section.current?.parentElement;
    return Math.max(0, (panel?.clientHeight ?? 0) - (panel?.dataset.git === 'true' ? 48 : 0));
  };
  const clampHeight = (value: number, maximum: number) => Math.round(Math.min(maximum, Math.max(140, value)));
  const saveHeight = (value?: number) => {
    setHeight(value);
    if (value === undefined) localStorage.removeItem(heightKey);
    else localStorage.setItem(heightKey, String(value));
  };
  const selected = hover ?? keyboard ?? (active >= 0 ? active : undefined);
  const selectedNote = selected === undefined ? undefined : graph?.nodes[selected];

  return <section ref={section} className="athena-graph" aria-label="Graph View"
    style={height !== undefined ? { '--athena-graph-height': `${height}px` } as CSSProperties : undefined}>
    <div className="graph-height-handle" role="separator" aria-orientation="horizontal" aria-label="Ajustar a altura do Graph View"
      title="Arraste para ajustar a altura. Duplo clique restaura o tamanho padrão." tabIndex={0}
      onPointerDown={event => {
        if (event.button !== 0 || !section.current) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        const start = section.current.getBoundingClientRect().height;
        heightDrag.current = { y: event.clientY, start, current: start, maximum: heightLimit() };
      }}
      onPointerMove={event => {
        const start = heightDrag.current;
        if (!start) return;
        start.current = clampHeight(start.start + start.y - event.clientY, start.maximum);
        setHeight(start.current);
      }}
      onPointerUp={() => {
        const start = heightDrag.current;
        if (start && start.start !== start.current) saveHeight(start.current);
        heightDrag.current = null;
      }}
      onLostPointerCapture={() => { heightDrag.current = null; }}
      onDoubleClick={() => saveHeight()}
      onKeyDown={event => {
        if (!section.current || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) return;
        event.preventDefault();
        const current = section.current.getBoundingClientRect().height;
        saveHeight(clampHeight(current + (event.key === 'ArrowUp' ? 16 : -16), heightLimit()));
      }} />
    <header className="panel-head">
      <b title={scope ? `Notas de ${scope}` : undefined}>Graph View{scope && <small> · {scope}</small>}</b>
      <button type="button" className="icon-button" aria-label="Centralizar grafo" title="Centralizar grafo" onClick={recenter}>⊙</button>
      <button type="button" className="icon-button" aria-label="Atualizar grafo" title="Atualizar grafo" onClick={onRefresh}><RefreshIcon size={15} /></button>
    </header>
    {error && <p className="graph-message" role="status">{error}</p>}
    {!hasNodes ? !error && <p className="graph-message">{graph ? `Nenhuma nota${scope ? ` em ${scope}` : ''}.` : 'Carregando conexões…'}</p>
      : <canvas ref={canvas} tabIndex={0} aria-label="Mapa de notas. Use as setas para selecionar uma nota e Enter para abrir."
        data-hover={hover !== undefined}
        onPointerDown={event => {
          if (event.button !== 0) return;
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = { start: { x: event.clientX, y: event.clientY }, camera: { ...camera.current }, moved: false, node: hit(event) };
        }}
        onPointerMove={event => {
          const start = drag.current;
          if (!start) { const next = hit(event); if (next !== hover) setHover(next); return; }
          const dx = event.clientX - start.start.x; const dy = event.clientY - start.start.y;
          if (!start.moved && Math.hypot(dx, dy) < 4) return;
          const state = sim.current;
          if (!state) return;
          if (!start.moved && start.node !== undefined) state.sim.heat(0.3);
          start.moved = true;
          if (start.node === undefined) {
            following.current = false;
            camera.current = { ...start.camera, x: start.camera.x - dx / start.camera.zoom, y: start.camera.y - dy / start.camera.zoom };
          } else {
            const rect = event.currentTarget.getBoundingClientRect();
            state.sim.pin(start.node, toWorld(event.clientX - rect.left, event.clientY - rect.top));
          }
          kick();
        }}
        onPointerUp={event => {
          const start = release();
          if (start && !start.moved) open(start.node ?? hit(event));
        }}
        onLostPointerCapture={() => { release(); }}
        onPointerLeave={() => { if (!drag.current) setHover(undefined); }}
        onBlur={() => setKeyboard(undefined)}
        onKeyDown={event => {
          const state = sim.current;
          if (event.key === 'Enter') { event.preventDefault(); open(keyboard ?? (active >= 0 ? active : undefined)); return; }
          if (event.key === 'Escape') { setKeyboard(undefined); return; }
          if (!graph || !state || !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
          event.preventDefault();
          const step = event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 1;
          const from = keyboard ?? (active >= 0 ? active : -1);
          const index = from < 0 ? 0 : (from + step + graph.nodes.length) % graph.nodes.length;
          setHover(undefined);
          setKeyboard(index);
          following.current = false;
          camera.current = { ...camera.current, ...state.sim.point(index), zoom: Math.max(camera.current.zoom, 1.2) };
        }} />}
    {hasNodes && <p className="graph-caption" aria-live="polite" title={selectedNote ? `${selectedNote.rootName} / ${selectedNote.path}` : undefined}>
      {selectedNote?.title ?? `${graph?.nodes.length} notas · ${graph?.edges.length} conexões`}
    </p>}
  </section>;
}

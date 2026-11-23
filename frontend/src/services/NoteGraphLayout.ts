export type Point = { x: number; y: number };

/** Ajustes das forças, no espírito dos controles do Obsidian (centro, repulsão, links). */
const linkDistance = 26;
const repel = -45;
/** Notas soltas ficam mais presas ao centro; grupos ligados só o bastante para não se afastarem. */
const orphanPull = 0.06;
const linkedPull = 0.03;
const theta2 = 0.81;
const velocityDecay = 0.4;
export const alphaMin = 0.002;
const alphaDecay = 1 - alphaMin ** (1 / 300);

type Quad = { x: number; y: number; size: number; mass: number; cx: number; cy: number; index: number; children: (Quad | undefined)[] | null };

/**
 * Simulação de forças contínua (atração pelos links, repulsão Barnes-Hut e leve puxão ao centro), como o
 * grafo do Obsidian: os pontos assentam aos poucos, reagem quando um deles é arrastado e notas soltas ficam
 * em volta do conjunto. Sem dependências; determinística para os mesmos dados.
 */
export class NoteGraphSimulation {
  readonly count: number;
  readonly x: Float64Array;
  readonly y: Float64Array;
  readonly vx: Float64Array;
  readonly vy: Float64Array;
  readonly neighbors: number[][];
  readonly edges: readonly [number, number][];
  private readonly pinned = new Map<number, Point>();
  private readonly bias: Float64Array;
  private readonly strength: Float64Array;
  alpha = 1;
  alphaTarget = 0;

  /** `previous` reaproveita posições de notas que já estavam no mapa (pela chave). */
  constructor(keys: readonly string[], edges: readonly [number, number][], previous?: Map<string, Point>) {
    const count = keys.length;
    this.count = count;
    this.edges = edges;
    this.x = new Float64Array(count); this.y = new Float64Array(count);
    this.vx = new Float64Array(count); this.vy = new Float64Array(count);
    this.neighbors = Array.from({ length: count }, () => []);
    for (const [a, b] of edges) { this.neighbors[a].push(b); this.neighbors[b].push(a); }
    this.bias = new Float64Array(edges.length);
    this.strength = new Float64Array(edges.length);
    for (const [i, [a, b]] of edges.entries()) {
      const da = this.neighbors[a].length; const db = this.neighbors[b].length;
      this.bias[i] = da / (da + db);
      this.strength[i] = 1 / Math.min(da, db);
    }

    // Notas já conhecidas voltam ao lugar; novas nascem perto de uma vizinha conhecida ou na espiral inicial.
    const known = new Uint8Array(count);
    for (const [i, key] of keys.entries()) {
      const point = previous?.get(key);
      if (point) { this.x[i] = point.x; this.y[i] = point.y; known[i] = 1; }
    }
    let reused = 0;
    for (let i = 0; i < count; i++) {
      if (known[i]) { reused++; continue; }
      const anchor = this.neighbors[i].find(n => known[n]);
      const angle = i * Math.PI * (3 - Math.sqrt(5));
      if (anchor !== undefined) {
        this.x[i] = this.x[anchor] + 12 * Math.cos(angle);
        this.y[i] = this.y[anchor] + 12 * Math.sin(angle);
      } else {
        const radius = 10 * Math.sqrt(0.5 + i);
        this.x[i] = radius * Math.cos(angle);
        this.y[i] = radius * Math.sin(angle);
      }
    }
    // Tudo no lugar: nada se move; com notas novas, só um leve reajuste.
    if (reused && count) this.alpha = reused === count ? 0 : Math.max(0.12, 1 - reused / count);
  }

  get running() { return this.alpha >= alphaMin || this.alphaTarget > 0; }
  degree(index: number) { return this.neighbors[index].length; }
  point(index: number): Point { return { x: this.x[index], y: this.y[index] }; }

  /** Posições atuais por chave, para manter o arranjo quando as notas mudam. */
  positions(keys: readonly string[]) {
    const out = new Map<string, Point>();
    for (const [i, key] of keys.entries()) out.set(key, this.point(i));
    return out;
  }

  pin(index: number, point: Point) {
    this.pinned.set(index, point);
    this.x[index] = point.x; this.y[index] = point.y; this.vx[index] = 0; this.vy[index] = 0;
  }
  release(index: number) { this.pinned.delete(index); }
  /** Reaquece a simulação para reagir a um arraste (alvo > 0) ou deixá-la esfriar (alvo 0). */
  heat(target: number, alpha = Math.max(this.alpha, target)) { this.alphaTarget = target; this.alpha = alpha; }

  bounds() {
    let minX = Infinity; let minY = Infinity; let maxX = -Infinity; let maxY = -Infinity;
    for (let i = 0; i < this.count; i++) {
      minX = Math.min(minX, this.x[i]); maxX = Math.max(maxX, this.x[i]);
      minY = Math.min(minY, this.y[i]); maxY = Math.max(maxY, this.y[i]);
    }
    return this.count ? { minX, minY, maxX, maxY } : { minX: 0, minY: 0, maxX: 0, maxY: 0 };
  }

  tick(steps = 1) {
    for (let step = 0; step < steps; step++) {
      this.alpha += (this.alphaTarget - this.alpha) * alphaDecay;
      const alpha = this.alpha;
      this.links(alpha);
      this.manyBody(alpha);
      const { x, y, vx, vy } = this;
      for (let i = 0; i < this.count; i++) {
        const pull = (this.neighbors[i].length ? linkedPull : orphanPull) * alpha;
        vx[i] -= x[i] * pull;
        vy[i] -= y[i] * pull;
        const fixed = this.pinned.get(i);
        if (fixed) { x[i] = fixed.x; y[i] = fixed.y; vx[i] = 0; vy[i] = 0; continue; }
        vx[i] *= 1 - velocityDecay; vy[i] *= 1 - velocityDecay;
        x[i] += vx[i]; y[i] += vy[i];
      }
    }
  }

  private links(alpha: number) {
    const { x, y, vx, vy } = this;
    for (const [i, [a, b]] of this.edges.entries()) {
      let dx = x[b] + vx[b] - x[a] - vx[a];
      let dy = y[b] + vy[b] - y[a] - vy[a];
      if (!dx && !dy) { dx = jiggle(i); dy = jiggle(i + 1); }
      const length = Math.hypot(dx, dy);
      const pull = (length - linkDistance) / length * alpha * this.strength[i];
      dx *= pull; dy *= pull;
      const bias = this.bias[i];
      vx[b] -= dx * bias; vy[b] -= dy * bias;
      vx[a] += dx * (1 - bias); vy[a] += dy * (1 - bias);
    }
  }

  /** Repulsão entre todas as notas aproximada por Barnes-Hut (O(n log n)). */
  private manyBody(alpha: number) {
    if (this.count < 2) return;
    const { minX, minY, maxX, maxY } = this.bounds();
    const size = Math.max(maxX - minX, maxY - minY, 1) * 1.0001;
    const root: Quad = quad(minX, minY, size);
    for (let i = 0; i < this.count; i++) insert(root, i, this.x, this.y, 0);
    accumulate(root, this.x, this.y);
    const { x, y, vx, vy } = this;
    const visit = (node: Quad, i: number) => {
      if (!node.mass) return;
      let dx = node.cx - x[i];
      let dy = node.cy - y[i];
      let distance2 = dx * dx + dy * dy;
      const leaf = !node.children;
      if (leaf && node.index === i && node.mass === 1) return;
      if (leaf || (node.size * node.size) / theta2 < distance2) {
        if (!distance2) { dx = jiggle(i); dy = jiggle(i + 7); distance2 = dx * dx + dy * dy; }
        if (distance2 < 1) distance2 = Math.sqrt(distance2);
        const mass = leaf && node.index === i ? node.mass - 1 : node.mass;
        const push = repel * mass * alpha / distance2;
        vx[i] += dx * push; vy[i] += dy * push;
        return;
      }
      for (const child of node.children!) if (child) visit(child, i);
    };
    for (let i = 0; i < this.count; i++) visit(root, i);
  }
}

const jiggle = (seed: number) => ((Math.sin(seed * 12.9898) * 43758.5453) % 1 - 0.5) * 1e-6;
const quad = (x: number, y: number, size: number): Quad => ({ x, y, size, mass: 0, cx: 0, cy: 0, index: -1, children: null });

function insert(node: Quad, index: number, xs: Float64Array, ys: Float64Array, depth: number) {
  if (!node.children && node.index < 0) { node.index = index; node.mass = 1; return; }
  // Pontos coincidentes (ou profundidade excessiva) ficam juntos na mesma folha.
  if (!node.children && (depth > 40 || (xs[node.index] === xs[index] && ys[node.index] === ys[index]))) { node.mass++; return; }
  if (!node.children) {
    const previous = node.index; const stacked = node.mass;
    node.children = [undefined, undefined, undefined, undefined];
    node.index = -1; node.mass = 0;
    for (let k = 0; k < stacked; k++) place(node, previous, xs, ys, depth);
  }
  place(node, index, xs, ys, depth);
}

function place(node: Quad, index: number, xs: Float64Array, ys: Float64Array, depth: number) {
  const half = node.size / 2;
  const right = xs[index] >= node.x + half ? 1 : 0;
  const bottom = ys[index] >= node.y + half ? 1 : 0;
  const slot = right + bottom * 2;
  const child = node.children![slot] ??= quad(node.x + right * half, node.y + bottom * half, half);
  insert(child, index, xs, ys, depth + 1);
}

function accumulate(node: Quad, xs: Float64Array, ys: Float64Array) {
  if (!node.children) {
    if (node.index >= 0) { node.cx = xs[node.index]; node.cy = ys[node.index]; }
    return;
  }
  let mass = 0; let cx = 0; let cy = 0;
  for (const child of node.children) {
    if (!child) continue;
    accumulate(child, xs, ys);
    mass += child.mass; cx += child.cx * child.mass; cy += child.cy * child.mass;
  }
  node.mass = mass; node.cx = mass ? cx / mass : 0; node.cy = mass ? cy / mass : 0;
}

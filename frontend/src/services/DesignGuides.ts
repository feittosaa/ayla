/** Guias do editor de designs (como no Canva): ao arrastar um elemento perto da margem, do centro
 * ou das bordas da página, ou das bordas e do centro de outro elemento, ele encaixa e a linha aparece.
 * Tudo em unidades da página; sem dependência do Fabric, para os testes. */

export type Box = { left: number; top: number; width: number; height: number };
export type Guides = { x: number[]; y: number[] };
export type Snap = { dx: number; dy: number; guides: Guides };

/** Margem de segurança da página: 5% do lado menor (mínimo de 8 px). */
export const marginOf = (width: number, height: number) => Math.max(8, Math.round(Math.min(width, height) * 0.05));

/** Linhas de encaixe da página e dos outros elementos, em cada eixo. */
export function snapLines(width: number, height: number, others: Box[]): Guides {
  const m = marginOf(width, height);
  const x = [0, m, width / 2, width - m, width];
  const y = [0, m, height / 2, height - m, height];
  for (const o of others) {
    x.push(o.left, o.left + o.width / 2, o.left + o.width);
    y.push(o.top, o.top + o.height / 2, o.top + o.height);
  }
  return { x, y };
}

/** Melhor encaixe de um eixo: começo, meio ou fim do elemento na linha mais próxima (até `threshold`). */
function axis(start: number, size: number, lines: number[], threshold: number) {
  let best: { delta: number; line: number } | null = null;
  for (const edge of [start, start + size / 2, start + size]) {
    for (const line of lines) {
      const delta = line - edge;
      if (Math.abs(delta) <= threshold && (!best || Math.abs(delta) < Math.abs(best.delta))) best = { delta, line };
    }
  }
  if (!best) return { delta: 0, lines: [] as number[] };
  // Depois do encaixe, mostra todas as linhas em que o elemento ficou alinhado.
  const moved = start + best.delta;
  const hit = lines.filter(line => [moved, moved + size / 2, moved + size].some(edge => Math.abs(edge - line) < 0.5));
  return { delta: best.delta, lines: [...new Set(hit)] };
}

export function snap(box: Box, lines: Guides, threshold: number): Snap {
  const x = axis(box.left, box.width, lines.x, threshold);
  const y = axis(box.top, box.height, lines.y, threshold);
  return { dx: x.delta, dy: y.delta, guides: { x: x.lines, y: y.lines } };
}

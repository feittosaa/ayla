import { useEffect, useState } from 'react';

// Diagramas Mermaid (```mermaid), como no Obsidian e no GitHub. A biblioteca só carrega quando
// aparece um diagrama, roda neste computador e desenha com as cores da Ayla.
let library: Promise<typeof import('mermaid').default> | null = null;
let counter = 0;
const cache = new Map<string, string>();

/** Cor resolvida (rgb) de uma variável do tema; o Mermaid não entende `var(--…)`. */
function color(name: string) {
  const probe = document.createElement('span');
  probe.style.color = `var(${name})`;
  probe.style.display = 'none';
  document.body.append(probe);
  const value = getComputedStyle(probe).color;
  probe.remove();
  return value;
}

/** Mistura duas cores rgb(): `amount` da primeira sobre a segunda (cor sólida, sem transparência). */
function mix(front: string, back: string, amount: number) {
  const parts = (value: string) => (value.match(/[\d.]+/g) ?? ['0', '0', '0']).slice(0, 3).map(Number);
  const [a, b] = [parts(front), parts(back)];
  return `rgb(${a.map((v, i) => Math.round(v * amount + b[i] * (1 - amount))).join(', ')})`;
}

async function renderDiagram(code: string): Promise<string> {
  const palette = ['--ayla-accent', '--ayla-text', '--ayla-text-soft', '--ayla-border', '--ayla-panel', '--ayla-bg'].map(color);
  const key = `${palette.join()}\n${code}`;
  const known = cache.get(key);
  if (known) return known;
  library ??= import('mermaid').then(module => module.default);
  const mermaid = await library;
  const [accent, text, textSoft, border, panel, background] = palette;
  // Nó tingido pelo destaque sobre o fundo, com o texto no próprio destaque (legível nos dois temas).
  const fill = mix(accent, background, 0.2); const edge = mix(accent, background, 0.35);
  const font = getComputedStyle(document.body).fontFamily;
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: 'strict',
    theme: 'base',
    fontFamily: font,
    flowchart: { curve: 'step', htmlLabels: true, padding: 16 },
    themeVariables: {
      fontFamily: font, fontSize: '14px', background,
      primaryColor: fill, primaryTextColor: accent, primaryBorderColor: edge,
      secondaryColor: panel, tertiaryColor: panel, mainBkg: fill, nodeBorder: edge, nodeTextColor: accent,
      lineColor: textSoft, textColor: text, edgeLabelBackground: background, clusterBkg: panel, clusterBorder: border,
      noteBkgColor: panel, noteTextColor: text, noteBorderColor: border,
    },
  });
  const id = `ayla-diagram-${++counter}`;
  // Com erro, o Mermaid pode deixar o rascunho do desenho solto na página.
  const { svg } = await mermaid.render(id, code).finally(() => { document.getElementById(`d${id}`)?.remove(); });
  if (cache.size > 40) cache.clear();
  cache.set(key, svg);
  return svg;
}

/** Diagrama desenhado; com erro de sintaxe, mostra o código e o aviso. */
export default function Diagram({ code }: { code: string }) {
  const [result, setResult] = useState<{ code: string; svg?: string; error?: string } | null>(null);
  useEffect(() => {
    let alive = true;
    renderDiagram(code)
      .then(svg => { if (alive) setResult({ code, svg }); })
      .catch(error => { if (alive) setResult({ code, error: error instanceof Error ? error.message.split('\n')[0] : 'Diagrama inválido' }); });
    return () => { alive = false; };
  }, [code]);
  const current = result?.code === code ? result : null;
  if (current?.error) return <div className="diagram diagram-error"><pre><code>{code}</code></pre><p>Não consegui desenhar o diagrama: {current.error}</p></div>;
  if (!current?.svg) return <div className="diagram diagram-loading" role="status">Desenhando o diagrama…</div>;
  return <div className="diagram" dangerouslySetInnerHTML={{ __html: current.svg }} />;
}

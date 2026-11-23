import { FIRST_VARIANT, MARKS, markUrl, useAppearance, type MarkName } from '../services/Appearance';

const urls = Object.fromEntries(Object.entries(MARKS).map(([name, list]) => [name, list.map(markUrl)])) as Record<MarkName, string[]>;

/** Marca de um espaço (Ayla, Athena, Iris, Hand) na variante escolhida em Aparência, na cor do texto. */
export default function BrandMark({ name, size = 22, variant }: { name: MarkName; size?: number; variant?: number }) {
  const appearance = useAppearance();
  const chosen = variant ?? (name === 'ayla' ? appearance.icon_ayla : name === 'athena' ? appearance.icon_athena : 1);
  const first = FIRST_VARIANT[name] ?? 1;
  const url = urls[name][Math.min(urls[name].length, Math.max(1, chosen - first + 1)) - 1];
  return <span className="brand-mark" aria-hidden="true" style={{ width: size, height: size, WebkitMaskImage: `url("${url}")`, maskImage: `url("${url}")` }} />;
}

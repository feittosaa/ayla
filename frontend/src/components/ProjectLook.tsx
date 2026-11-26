import { useEffect, useRef, useState } from 'react';
import { errorText, native, type Project } from '../services/Desktop';
import { PROJECT_COLORS } from '../services/ProjectLook';
type Icon = { name: string; svg: string };

/** Desenho de um ícone do Lucide na cor do texto (máscara), sem injetar o SVG na página. */
export function IconMask({ svg, size }: { svg: string; size: number }) {
  const url = `url("data:image/svg+xml;utf8,${encodeURIComponent(svg)}")`;
  return <span className="icon-mask" aria-hidden="true" style={{ width: size, height: size, WebkitMaskImage: url, maskImage: url }} />;
}

/** Nome, ícone (do pacote de recursos da Iris, o mesmo dos designs) e cor de fundo do projeto. */
export default function ProjectLook({ project, refresh, preview }: { project: Project; refresh: () => Promise<void>; preview: (look: Pick<Project, 'icon' | 'icon_svg' | 'color' | 'name'>) => React.ReactNode }) {
  const [name, setName] = useState(project.name);
  const [icon, setIcon] = useState<Icon | null>(project.icon && project.icon_svg ? { name: project.icon, svg: project.icon_svg } : null);
  const [color, setColor] = useState(project.color ?? '');
  const [query, setQuery] = useState('');
  const [icons, setIcons] = useState<Icon[] | null>(null);
  const [more, setMore] = useState(false);
  const [missing, setMissing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const search = useRef(0);
  useEffect(() => {
    const id = ++search.current;
    const timer = setTimeout(() => {
      void native<{ items: Icon[]; more: boolean; total: number }>('design_icons', { query, offset: 0 }).then(page => {
        if (search.current !== id) return;
        setIcons(page.items); setMore(page.more); setMissing(page.total === 0 && !query.trim());
      }, e => setError(errorText(e)));
    }, 200);
    return () => clearTimeout(timer);
  }, [query]);
  async function loadMore() {
    const page = await native<{ items: Icon[]; more: boolean }>('design_icons', { query, offset: icons?.length ?? 0 }).catch(e => { setError(errorText(e)); return null; });
    if (page) { setIcons(prev => [...(prev ?? []), ...page.items]); setMore(page.more); }
  }
  const changed = name.trim() !== project.name || (icon?.name ?? null) !== (project.icon ?? null) || color !== (project.color ?? '');
  async function save() {
    setSaving(true); setError(''); setSaved(false);
    try {
      if (name.trim() !== project.name) await native('update_project', { id: project.id, name: name.trim(), description: project.description });
      await native('set_project_look', { id: project.id, icon: icon?.name ?? null, color: color || null });
      await refresh(); setSaved(true);
    } catch (e) { setError(errorText(e)); } finally { setSaving(false); }
  }
  return <div className="group-card project-look">
    <div className="group-row stacked">
      <div className="project-look-top">
        {preview({ name: name.trim() || project.name, icon: icon?.name ?? null, icon_svg: icon?.svg ?? null, color: color || null })}
        <label className="project-look-name"><span className="row-title">Nome</span>
          <input value={name} maxLength={80} onChange={e => { setName(e.target.value); setSaved(false); }} /></label>
      </div>
      <span className="row-hint">A pasta do projeto na Athena (Projetos/{project.folder ?? name}) acompanha o nome.</span>
    </div>
    <div className="group-row stacked">
      <span className="row-title">Cor de fundo</span>
      <div className="project-colors" role="radiogroup" aria-label="Cor de fundo">
        <button type="button" role="radio" aria-checked={color === ''} className="project-color auto" onClick={() => { setColor(''); setSaved(false); }}>Automática</button>
        <button type="button" role="radio" aria-checked={color === 'none'} className="project-color none" onClick={() => { setColor('none'); setSaved(false); }}>Sem cor</button>
        {PROJECT_COLORS.map(c => <button key={c} type="button" role="radio" aria-checked={color === c} aria-label={c} className="project-color swatch" style={{ background: c }} onClick={() => { setColor(c); setSaved(false); }} />)}
        <label className="project-color custom" title="Outra cor"><input type="color" value={/^#[0-9a-f]{6}$/i.test(color) ? color : '#7d9bff'} onChange={e => { setColor(e.target.value.toLowerCase()); setSaved(false); }} />Outra</label>
      </div>
    </div>
    <div className="group-row stacked">
      <span className="row-title">Ícone</span>
      {missing ? <span className="row-hint">Os ícones vêm do pacote de recursos da Iris (o mesmo dos designs). Baixe-o em Iris › Designs para escolher um ícone; sem ele, o projeto usa as iniciais.</span> : <>
        <div className="project-icon-bar">
          <input type="search" placeholder="Buscar ícone (em inglês: book, sword, home…)" value={query} onChange={e => setQuery(e.target.value)} aria-label="Buscar ícone" />
          {icon && <button type="button" className="ghost-button" onClick={() => { setIcon(null); setSaved(false); }}>Usar as iniciais</button>}
        </div>
        <div className="project-icons" role="listbox" aria-label="Ícones">
          {icons === null ? <span className="row-hint">Carregando…</span> : !icons.length ? <span className="row-hint">Nenhum ícone com esse nome.</span>
            : icons.map(item => <button key={item.name} type="button" role="option" aria-selected={icon?.name === item.name} title={item.name} aria-label={item.name}
                onClick={() => { setIcon(item); setSaved(false); }}><IconMask svg={item.svg} size={20} /></button>)}
        </div>
        {more && <button type="button" className="ghost-button" onClick={() => void loadMore()}>Mostrar mais ícones</button>}
      </>}
    </div>
    {error && <p role="alert" className="app-error">{error}</p>}
    <div className="group-row"><span className="row-hint">{saved ? 'Salvo.' : ''}</span>
      <button className="primary-action" disabled={!changed || saving || !name.trim()} onClick={() => void save()}>{saving ? 'Salvando…' : 'Salvar'}</button></div>
  </div>;
}

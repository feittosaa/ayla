import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { native, type Preferences } from '../services/Desktop';

type Options = Preferences['desktop'];
type Theme = { colors: Record<string,string>; font: string | null; radius: number };
type View = { quick: boolean; pending: number | null; theme: Theme | null };
export function useDesktopView(onQuick: () => void, onError: (reason: string) => void) {
  const [view, setView] = useState<View>({ quick: false, pending: null, theme: null });
  const markShown = useCallback((generation: number) => {
    setView(current => current.pending === generation ? { ...current, pending: null } : current);
  }, []);
  useEffect(() => {
    let alive = true; let generation = 0;
    const read = async () => {
      const ticket = ++generation;
      const value = await native<View>('desktop_view');
      if (alive && generation === ticket) { setView(value); if (value.quick) onQuick(); }
    };
    const events = listen('desktop-view-changed', () => void read().catch(e => onError(String(e))));
    const warning = listen<string>('desktop-warning', e => onError(e.payload));
    void events.then(read).catch(e => onError(String(e)));
    return () => { alive = false; for (const item of [events, warning]) void item.then(stop => stop()).catch(() => {}); };
  }, [onQuick,onError]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.key !== 'Escape') return;
      if (view.quick || !(e.target instanceof HTMLInputElement)) {
        void native('desktop_action', {action:view.quick ? 'hide' : 'restore'}).catch(e => onError(String(e)));
      }
    };
    window.addEventListener('keydown',key);
    return () => window.removeEventListener('keydown',key);
  }, [view.quick,onError]);
  return { view, markShown };
}

// Só a janela compacta recebe estas variáveis. Os controles usam a mesma paleta do painel.
export function quickStyle(theme: Theme | null, p: Options): CSSProperties {
  const c = p.system_theme ? theme?.colors || {} : {};
  const bg = p.system_theme ? c.background || 'Canvas' : p.panel_color;
  const accent = p.system_theme ? c.primary || 'AccentColor' : p.panel_accent;
  const ink = (hex: string) => {
    if (!/^#[a-f0-9]{6}$/i.test(hex)) return 'CanvasText';
    const [r,g,b] = [1,3,5].map(i => { const v = parseInt(hex.slice(i,i+2),16)/255; return v <= .04045 ? v/12.92 : ((v+.055)/1.055)**2.4; });
    return .2126*r+.7152*g+.0722*b > .179 ? '#111111' : '#ffffff';
  };
  const fg = c.on_surface || ink(bg);
  const mix = (pct: number) => `color-mix(in srgb, ${fg} ${pct}%, ${bg})`;
  const family = (p.system_theme ? theme?.font : p.panel_font)?.trim();
  return {
    '--ayla-bg':bg, '--ayla-accent':accent, '--ayla-text':fg, '--ayla-on-accent':c.on_primary || ink(accent),
    '--ayla-sidebar':c.surface_container_low || mix(4), '--ayla-panel':c.surface_container || mix(8),
    '--ayla-control':c.surface_container_high || mix(12), '--ayla-hover':c.surface_container_highest || mix(16),
    '--ayla-selected':c.primary_container || mix(20), '--ayla-border':c.outline || mix(46),
    '--ayla-border-soft':c.outline_variant || mix(24), '--ayla-text-soft':c.on_surface_variant || mix(80),
    '--ayla-text-faint':c.on_surface_variant || mix(72), '--ayla-disabled':c.outline || mix(55),
    '--ayla-accent-soft':accent, '--ayla-accent-ring':c.primary_container || mix(20), '--ayla-error':c.error || '#ce5151',
    '--ayla-font-main':family && family !== 'system-ui' ? `"${CSS.escape(family)}", system-ui, sans-serif` : 'system-ui, sans-serif',
    '--panel-radius':`${p.system_theme ? theme?.radius ?? 18 : p.panel_radius}px`,
  } as CSSProperties;
}

export function DesktopSettings({value, saving, save, run}: {
  value: Options; saving: boolean; save: (value: Options) => void; run: (action: string) => void;
}) {
  const [custom,setCustom] = useState(value);
  useEffect(() => setCustom(value),[value.panel_color,value.panel_accent,value.panel_font,value.panel_radius]); // só sincronizar o editor quando o estilo salvo mudar
  return <section id="desktop" className="settings-group"><h2>Integração com o sistema</h2><div className="group-card">
    {([['autostart','Iniciar com a sessão'],['background','Continuar em segundo plano ao fechar'],['system_theme','Usar a aparência do sistema na visão rápida']] as const).map(([key,label]) =>
      <label key={key} className="group-row"><span className="row-text">{label}</span><input className="switch" role="switch" type="checkbox" checked={value[key]} disabled={saving} onChange={e => save({...value,[key]:e.target.checked})} /></label>)}
    {!value.system_theme && <div className="group-row stacked panel-editor">
      <label>Cor de fundo<input type="color" value={custom.panel_color} onChange={e=>setCustom({...custom,panel_color:e.target.value})} /></label>
      <label>Cor de destaque<input type="color" value={custom.panel_accent} onChange={e=>setCustom({...custom,panel_accent:e.target.value})} /></label>
      <label>Fonte<input maxLength={100} value={custom.panel_font} onChange={e=>setCustom({...custom,panel_font:e.target.value})} /></label>
      <label>Cantos ({custom.panel_radius} px)<input type="range" min="0" max="48" value={custom.panel_radius} onChange={e=>setCustom({...custom,panel_radius:Number(e.target.value)})} /></label>
      <button disabled={saving} onClick={()=>save({...value,panel_color:custom.panel_color,panel_accent:custom.panel_accent,panel_font:custom.panel_font,panel_radius:custom.panel_radius})}>Salvar aparência</button>
    </div>}
    <div className="group-row row-actions"><button onClick={()=>run('quick')}>Abrir visão rápida</button><button onClick={()=>run('quit')}>Encerrar Ayla</button></div>
  </div></section>;
}

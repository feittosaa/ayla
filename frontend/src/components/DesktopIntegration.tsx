import { useState } from 'react';
import type { Options } from '../services/DesktopView';

export function DesktopSettings({value, saving, save, run}: {
  value: Options; saving: boolean; save: (value: Options) => void; run: (action: string) => void;
}) {
  const [custom,setCustom] = useState(value);
  // Só sincroniza o editor quando o estilo salvo muda (ajuste durante a renderização, sem efeito).
  const style = [value.panel_color,value.panel_accent,value.panel_font,value.panel_radius].join('|');
  const [synced,setSynced] = useState(style);
  if (synced !== style) { setSynced(style); setCustom(value); }
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

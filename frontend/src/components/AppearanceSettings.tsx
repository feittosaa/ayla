import { useRef, useState } from 'react';
import BrandMark from './BrandMark';
import { ACCENTS, BACKGROUNDS, DEFAULT_APPEARANCE, ROUNDNESS, type Appearance, type Mode } from '../services/Appearance';
import { errorText, native } from '../services/Desktop';

const modes: { id: Appearance['mode']; label: string }[] = [{ id: 'escuro', label: 'Escuro' }, { id: 'claro', label: 'Claro' }, { id: 'sistema', label: 'Seguir o sistema' }];

/** Configurações › Aparência. Cada ajuste aparece na hora e é salvo logo depois (só a aparência). */
export default function AppearanceSettings({ saved, onPreview, onSaved }: {
  saved: Appearance; onPreview: (value: Appearance) => void; onSaved: () => void;
}) {
  const [value, setValue] = useState(saved);
  const [error, setError] = useState('');
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const queue = useRef(Promise.resolve());
  function change(values: Partial<Appearance>) {
    const next = { ...value, ...values };
    setValue(next); onPreview(next); setError('');
    clearTimeout(timer.current);
    // Arrastar o seletor de cor gera muitos eventos: salva quando a pessoa para.
    timer.current = setTimeout(() => {
      queue.current = queue.current.then(() => native('save_appearance', { appearance: next }))
        .then(onSaved, e => setError(errorText(e)));
    }, 350);
  }
  const backgroundRow = (mode: Mode) => {
    const key = mode === 'escuro' ? 'dark_background' : 'light_background';
    return <div className="group-row stacked" key={mode}>
      <span className="row-text"><span className="row-title">{value.mode === 'sistema' ? `Fundo no modo ${mode}` : 'Fundo'}</span>
        <span className="row-hint">As superfícies seguem o tom do fundo; a Ayla ajusta cores claras ou escuras demais para manter o texto legível.</span></span>
      <div className="swatches">
        {BACKGROUNDS[mode].map(item => <button key={item.color} type="button" className="swatch labeled" aria-pressed={value[key] === item.color} title={item.name}
          onClick={() => change({ [key]: item.color })}><span style={{ background: item.color }} />{item.name}</button>)}
        <label className="swatch labeled custom" title="Escolher outra cor"><input type="color" value={value[key]} onChange={e => change({ [key]: e.target.value })} /><span style={{ background: value[key] }} />Outra cor</label>
      </div>
    </div>;
  };
  return <section id="aparencia" className="settings-group" aria-labelledby="aparencia-title"><h2 id="aparencia-title">Aparência</h2>
    <div className="group-card">
      <fieldset className="group-row stacked"><legend className="row-title">Modo</legend>
        <div className="segmented three">{modes.map(item => <label key={item.id}><input type="radio" name="appearance-mode" checked={value.mode === item.id} onChange={() => change({ mode: item.id })} /><span>{item.label}</span></label>)}</div>
      </fieldset>
      <div className="group-row stacked">
        <span className="row-text"><span className="row-title">Cor de destaque</span><span className="row-hint">Botões principais, seleção e indicadores.</span></span>
        <div className="swatches">
          {ACCENTS.map(color => <button key={color} type="button" className="swatch" aria-label={`Destaque ${color}`} aria-pressed={value.accent === color} onClick={() => change({ accent: color })}><span style={{ background: color }} /></button>)}
          <label className="swatch custom" title="Escolher outra cor" aria-label="Outra cor de destaque"><input type="color" value={value.accent} onChange={e => change({ accent: e.target.value })} /><span style={{ background: value.accent }} /></label>
        </div>
      </div>
      {(value.mode === 'sistema' ? (['escuro', 'claro'] as Mode[]) : [value.mode as Mode]).map(backgroundRow)}
      <fieldset className="group-row stacked"><legend className="row-title">Quinas</legend>
        <div className="segmented">{ROUNDNESS.map(item => <label key={item.value}><input type="radio" name="appearance-round" checked={value.roundness === item.value} onChange={() => change({ roundness: item.value })} /><span>{item.label}</span></label>)}</div>
      </fieldset>
      <label className="group-row stacked">
        <span className="row-text"><span className="row-title">Transparência da janela: {100 - value.opacity}%</span>
          <span className="row-hint">Deixa o fundo da janela translúcido; textos e controles continuam opacos. O desfoque por trás depende do sistema (no Hyprland, o blur da janela).</span></span>
        <input type="range" min={0} max={60} step={5} value={100 - value.opacity} onChange={e => change({ opacity: 100 - Number(e.target.value) })} />
      </label>
      <div className="group-row stacked">
        <span className="row-text"><span className="row-title">Ícones</span><span className="row-hint">O ícone da Ayla também vira o da janela e do menu de aplicativos, na cor de destaque.</span></span>
        <div className="icon-choices" role="group" aria-label="Ícone da Ayla"><span className="icon-choices-label">Ayla</span>
          {[1, 2, 3].map(n => <button key={n} type="button" className="icon-choice" aria-label={`Ícone ${n} da Ayla`} aria-pressed={value.icon_ayla === n} onClick={() => change({ icon_ayla: n })}><BrandMark name="ayla" variant={n} size={40} /></button>)}
        </div>
        <div className="icon-choices" role="group" aria-label="Ícone da Athena"><span className="icon-choices-label">Athena</span>
          {[3, 4].map((n, i) => <button key={n} type="button" className="icon-choice" aria-label={`Ícone ${i + 1} da Athena`} aria-pressed={value.icon_athena === n} onClick={() => change({ icon_athena: n })}><BrandMark name="athena" variant={n} size={40} /></button>)}
        </div>
        <div className="icon-choices" role="group" aria-label="Outros ícones"><span className="icon-choices-label">Iris</span>
          <span className="icon-choice" aria-pressed="true"><BrandMark name="iris" size={40} /></span>
          <span className="icon-choices-label">Hand</span>
          <span className="icon-choice soon" title="Hand · órgão futuro"><BrandMark name="hand" size={40} /></span>
        </div>
      </div>
      <div className="group-row">
        <span className="row-text"><span className="row-title">Aparência original</span><span className="row-hint">Volta ao modo escuro, ao laranja e aos ícones padrão.</span></span>
        <button type="button" className="ghost-button" onClick={() => change(DEFAULT_APPEARANCE)}>Restaurar o padrão</button>
      </div>
      {error && <p role="alert" className="app-error">{error}</p>}
    </div>
  </section>;
}

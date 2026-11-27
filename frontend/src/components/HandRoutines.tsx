import { useCallback, useEffect, useMemo, useState } from 'react';
import Confirm from './Confirm';
import { PlusIcon, TrashIcon } from './Icons';
import { errorText, native } from '../services/Desktop';

type Gatilho = { tipo: 'intervalo'; minutos: number } | { tipo: 'horario'; horas: string[]; dias: number[] } | { tipo: 'ao_entrar' };
type Compara = 'sempre' | 'abaixo' | 'acima' | 'caiu' | 'mudou';
type Condicao =
  | { tipo: 'preco'; url: string; produto: string; compara: Compara; valor: number | null }
  | { tipo: 'janela'; de: string; ate: string }
  | { tipo: 'dia_semana'; dias: number[] }
  | { tipo: 'programa_aberto'; nome: string; aberto: boolean }
  | { tipo: 'arquivo_existe'; caminho: string; existe: boolean }
  | { tipo: 'uma_vez_por_dia' };
type Acao =
  | { tipo: 'notificar'; titulo: string; texto: string }
  | { tipo: 'mensagem_ayla'; pedido: string }
  | { tipo: 'macro'; id: string; nome: string }
  | { tipo: 'comando'; comando: string; aprovado: string };
type Execucao = { quando: number; disparou: boolean; detalhe: string; preco: number | null };
type Rotina = { id: string; nome: string; ativa: boolean; gatilho: Gatilho; condicoes: { modo: 'todas' | 'qualquer'; itens: Condicao[] }; acoes: Acao[]; nuvem: boolean;
  ultima?: number | null; ultimo_disparo?: number | null; ultimo_preco?: number | null; resumo?: string; proxima?: number | null; historico?: Execucao[] };
type Macro = { id: string; nome: string };

const DIAS = ['seg', 'ter', 'qua', 'qui', 'sex', 'sáb', 'dom'];
const reais = (v: number) => v.toLocaleString('pt-BR', { style: 'currency', currency: 'BRL' });
// Os horários das rotinas são locais, guardados como se fossem UTC (como os lembretes).
const quando = (s: number) => new Date(s * 1000).toLocaleString('pt-BR', { timeZone: 'UTC', dateStyle: 'short', timeStyle: 'short' });
const newId = () => `r-${Date.now().toString(16)}${Math.random().toString(16).slice(2, 6)}`;

/** Gráfico simples do preço nas checagens. */
function Sparkline({ points }: { points: number[] }) {
  if (points.length < 2) return null;
  const [min, max] = [Math.min(...points), Math.max(...points)];
  const span = max - min || 1;
  const d = points.map((p, i) => `${(i / (points.length - 1)) * 200},${38 - ((p - min) / span) * 34}`).join(' ');
  return <svg className="hand-spark" viewBox="0 0 200 40" preserveAspectRatio="none" role="img" aria-label={`Preço de ${reais(min)} a ${reais(max)}`}><polyline points={d} fill="none" stroke="currentColor" strokeWidth="1.6" vectorEffect="non-scaling-stroke" /></svg>;
}

/** Rotinas: gatilho (intervalo, horários, ao entrar), condições (todas ou qualquer) e ações. */
export default function HandRoutines({ active, fail }: { active: boolean; fail: (e: unknown) => void }) {
  const [data, setData] = useState<{ rotinas: Rotina[]; nuvem: string | null } | null>(null);
  const [editing, setEditing] = useState<Rotina | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [notice, setNotice] = useState('');
  const [testing, setTesting] = useState('');
  const load = useCallback(() => native<{ rotinas: Rotina[]; nuvem: string | null }>('rotinas_lista').then(setData).catch(fail), [fail]);
  useEffect(() => { if (!active) return; void load(); const t = window.setInterval(() => void load(), 30000); return () => window.clearInterval(t); }, [active, load]);
  async function toggle(r: Rotina) {
    try { await native('rotina_ativar', { id: r.id, ativa: !r.ativa }); setNotice(r.ativa ? `“${r.nome}” pausada.` : `“${r.nome}” ativada.`); await load(); } catch (e) { fail(e); }
  }
  async function test(r: Rotina) {
    setTesting(r.id); setNotice('');
    try { const ex = await native<Execucao>('rotina_testar', { id: r.id }); setNotice(`${r.nome}: ${ex.disparou ? 'rodou' : 'condições não valeram'}${ex.detalhe ? ` — ${ex.detalhe}` : ''}.`); setOpen(r.id); await load(); }
    catch (e) { fail(e); } finally { setTesting(''); }
  }
  if (editing) return <RoutineEditor initial={editing} nuvem={data?.nuvem ?? null} close={saved => { setEditing(null); if (saved) { setNotice(`Rotina “${saved}” salva.`); void load(); } }} />;
  return <div className="hand-panel">
    <p className="panel-muted">Rotinas rodam sozinhas enquanto a Ayla estiver aberta ou em segundo plano: a cada intervalo, em horários ou ao entrar na sessão, se as condições valerem. Também dá para pedir na conversa: "checa o preço disto a cada 1 hora".</p>
    <div className="row-actions"><button className="primary-action" onClick={() => setEditing({ id: newId(), nome: '', ativa: true, gatilho: { tipo: 'intervalo', minutos: 60 }, condicoes: { modo: 'todas', itens: [] }, acoes: [{ tipo: 'notificar', titulo: '', texto: '' }], nuvem: false })}><PlusIcon size={15} />Nova rotina</button></div>
    {notice && <p role="status" className="hand-done">{notice}</p>}
    {data && !data.rotinas.length && <p className="panel-muted">Nenhuma rotina ainda.</p>}
    <ul className="hand-routines">{data?.rotinas.map(r => {
      const prices = (r.historico ?? []).filter(h => h.preco != null).map(h => h.preco as number).reverse();
      return <li key={r.id} className={r.ativa ? undefined : 'off'}>
        <div className="hand-routine-head">
          <label className="hand-switch" title={r.ativa ? 'Pausar' : 'Ativar'}><input type="checkbox" role="switch" checked={r.ativa} onChange={() => void toggle(r)} /><span aria-hidden="true" /></label>
          <span className="hand-bind-what"><b>{r.nome}</b><small>{r.resumo}</small>
            <small>{r.ativa ? (r.proxima ? `próxima: ${quando(r.proxima)}` : r.gatilho.tipo === 'ao_entrar' ? 'na próxima vez que entrar na sessão' : '') : 'pausada'}{r.ultima ? ` · última: ${quando(r.ultima)}` : ''}{r.ultimo_preco ? ` · preço: ${reais(r.ultimo_preco)}` : ''}</small></span>
          {prices.length > 1 && <Sparkline points={prices} />}
          <span className="row-actions">
            <button className="ghost-button" disabled={!!testing} onClick={() => void test(r)}>{testing === r.id ? 'Rodando…' : 'Testar agora'}</button>
            <button className="ghost-button" onClick={() => setEditing(r)}>Editar</button>
            <button className="ghost-button" aria-expanded={open === r.id} onClick={() => setOpen(open === r.id ? null : r.id)}>Histórico</button>
            <Confirm className="icon-button" ariaLabel={`Apagar ${r.nome}`} title="Apagar rotina" question={`Apagar a rotina “${r.nome}” e o histórico dela?`} confirm="Apagar"
              onConfirm={() => void native('rotina_apagar', { id: r.id }).then(() => { setNotice('Rotina apagada.'); void load(); }).catch(fail)}><TrashIcon size={15} /></Confirm>
          </span>
        </div>
        {open === r.id && <ul className="hand-routine-log">{(r.historico ?? []).length ? r.historico!.map((h, i) => <li key={i} className={h.disparou ? 'fired' : undefined}>
          <code>{quando(h.quando)}</code><span>{h.disparou ? '● rodou' : '○ não rodou'}</span><small>{h.detalhe}</small></li>) : <li><small>Ainda não rodou.</small></li>}</ul>}
      </li>;
    })}</ul>
  </div>;
}

function RoutineEditor({ initial, nuvem, close }: { initial: Rotina; nuvem: string | null; close: (saved?: string) => void }) {
  const [r, setR] = useState<Rotina>(initial);
  const [macros, setMacros] = useState<Macro[]>([]);
  const [error, setError] = useState('');
  useEffect(() => { void native<{ macros: Macro[] }>('hand_macros').then(m => setMacros(m.macros)).catch(() => {}); }, []);
  const set = (patch: Partial<Rotina>) => setR(prev => ({ ...prev, ...patch }));
  const setCond = (i: number, c: Condicao) => set({ condicoes: { ...r.condicoes, itens: r.condicoes.itens.map((x, j) => j === i ? c : x) } });
  const setAcao = (i: number, a: Acao) => set({ acoes: r.acoes.map((x, j) => j === i ? a : x) });
  const comandos = useMemo(() => r.acoes.filter((a): a is Extract<Acao, { tipo: 'comando' }> => a.tipo === 'comando' && a.comando.trim() !== '' && a.aprovado !== a.comando), [r.acoes]);
  const temPreco = r.condicoes.itens.some(c => c.tipo === 'preco');
  async function save() {
    setError('');
    // Salvar aprova os comandos que a pessoa acabou de conferir (a pergunta vem antes, se houver).
    const item: Rotina = { ...r, nome: r.nome.trim(), acoes: r.acoes.map(a => a.tipo === 'comando' ? { ...a, aprovado: a.comando } : a) };
    const { resumo: _r, proxima: _p, historico: _h, ...plain } = item;
    void _r; void _p; void _h;
    try { await native('rotina_salvar', { item: plain }); close(item.nome); } catch (e) { setError(errorText(e)); }
  }
  const g = r.gatilho;
  const dayChips = (dias: number[], onChange: (d: number[]) => void) => <span className="hand-options">{DIAS.map((d, i) =>
    <label key={d} className={dias.includes(i) ? 'hand-chip on' : 'hand-chip'}><input type="checkbox" checked={dias.includes(i)} onChange={e => onChange(e.target.checked ? [...dias, i].sort() : dias.filter(x => x !== i))} />{d}</label>)}</span>;
  return <div className="hand-panel">
    <section className="hand-plan hand-routine-editor" aria-label="Rotina">
      <input value={r.nome} autoFocus placeholder="Nome (ex.: Preço do monitor, Bom dia)" aria-label="Nome da rotina" onChange={e => set({ nome: e.target.value })} />

      <h3 className="hand-step-title">Quando</h3>
      <div className="hand-search">
        <select aria-label="Gatilho" value={g.tipo} onChange={e => set({ gatilho: e.target.value === 'intervalo' ? { tipo: 'intervalo', minutos: 60 } : e.target.value === 'horario' ? { tipo: 'horario', horas: ['08:00'], dias: [] } : { tipo: 'ao_entrar' } })}>
          <option value="intervalo">A cada…</option><option value="horario">Em horários</option><option value="ao_entrar">Ao entrar na sessão (ligar o PC)</option>
        </select>
        {g.tipo === 'intervalo' && <label className="hand-inline"><input className="hand-number" type="number" min={1} value={g.minutos >= 60 && g.minutos % 60 === 0 ? g.minutos / 60 : g.minutos} aria-label="Intervalo"
            onChange={e => set({ gatilho: { tipo: 'intervalo', minutos: Math.max(1, Number(e.target.value)) * (g.minutos >= 60 && g.minutos % 60 === 0 ? 60 : 1) } })} />
          <select aria-label="Unidade" value={g.minutos >= 60 && g.minutos % 60 === 0 ? 'h' : 'min'} onChange={e => set({ gatilho: { tipo: 'intervalo', minutos: e.target.value === 'h' ? Math.max(1, Math.round(g.minutos / 60)) * 60 : Math.max(15, g.minutos) } })}><option value="min">minutos</option><option value="h">horas</option></select></label>}
        {g.tipo === 'horario' && <>
          <input value={g.horas.join(', ')} aria-label="Horários" placeholder="08:00, 18:30" spellCheck={false} onChange={e => set({ gatilho: { ...g, horas: e.target.value.split(/[,\s]+/).filter(Boolean) } })} />
          {dayChips(g.dias, dias => set({ gatilho: { ...g, dias } }))}</>}
      </div>
      {g.tipo === 'horario' && <small className="panel-muted">Sem dias marcados: todos os dias.</small>}
      {g.tipo === 'ao_entrar' && <small className="panel-muted">Logo depois de entrar na sessão (a Ayla abre junto com o sistema). Para "de manhã", junte a condição de horário; para não repetir se você reiniciar, "uma vez por dia".</small>}

      <h3 className="hand-step-title">Se <select aria-label="Como juntar" value={r.condicoes.modo} onChange={e => set({ condicoes: { ...r.condicoes, modo: e.target.value as 'todas' | 'qualquer' } })}><option value="todas">todas valerem</option><option value="qualquer">qualquer uma valer</option></select></h3>
      {!r.condicoes.itens.length && <small className="panel-muted">Sem condições: roda sempre que o gatilho disparar.</small>}
      <ul className="hand-cond-list">{r.condicoes.itens.map((c, i) => <li key={i}>
        {c.tipo === 'preco' && <>
          <span className="hand-verb">Preço</span>
          <input className="hand-command" value={c.url} placeholder="https://loja.com.br/produto" aria-label="Link da loja" spellCheck={false} onChange={e => setCond(i, { ...c, url: e.target.value.trim() })} />
          <input value={c.produto} placeholder="Produto (para a nuvem)" aria-label="Produto" onChange={e => setCond(i, { ...c, produto: e.target.value })} />
          <select aria-label="Regra" value={c.compara} onChange={e => setCond(i, { ...c, compara: e.target.value as Compara })}>
            <option value="sempre">só acompanhar</option><option value="caiu">se cair</option><option value="mudou">se mudar</option><option value="abaixo">abaixo de</option><option value="acima">acima de</option></select>
          {(c.compara === 'abaixo' || c.compara === 'acima') && <label className="hand-inline">R$ <input className="hand-number" type="number" min={0} step={0.01} value={c.valor ?? ''} aria-label="Valor" onChange={e => setCond(i, { ...c, valor: e.target.value ? Number(e.target.value) : null })} /></label>}
        </>}
        {c.tipo === 'janela' && <><span className="hand-verb">Entre</span><input className="hand-time" type="time" value={c.de} aria-label="De" onChange={e => setCond(i, { ...c, de: e.target.value })} /> e <input className="hand-time" type="time" value={c.ate} aria-label="Até" onChange={e => setCond(i, { ...c, ate: e.target.value })} /></>}
        {c.tipo === 'dia_semana' && <><span className="hand-verb">Nos dias</span>{dayChips(c.dias, dias => setCond(i, { ...c, dias }))}</>}
        {c.tipo === 'programa_aberto' && <><span className="hand-verb">Programa</span><input value={c.nome} placeholder="steam, discord…" aria-label="Programa" onChange={e => setCond(i, { ...c, nome: e.target.value })} />
          <select aria-label="Estado" value={c.aberto ? 's' : 'n'} onChange={e => setCond(i, { ...c, aberto: e.target.value === 's' })}><option value="s">aberto</option><option value="n">fechado</option></select></>}
        {c.tipo === 'arquivo_existe' && <><span className="hand-verb">Arquivo</span><input className="hand-command" value={c.caminho} placeholder="/home/…/arquivo" aria-label="Caminho" spellCheck={false} onChange={e => setCond(i, { ...c, caminho: e.target.value })} />
          <select aria-label="Estado" value={c.existe ? 's' : 'n'} onChange={e => setCond(i, { ...c, existe: e.target.value === 's' })}><option value="s">existe</option><option value="n">não existe</option></select></>}
        {c.tipo === 'uma_vez_por_dia' && <span className="hand-verb">Uma vez por dia</span>}
        <button type="button" className="icon-button" aria-label="Tirar condição" onClick={() => set({ condicoes: { ...r.condicoes, itens: r.condicoes.itens.filter((_, j) => j !== i) } })}><TrashIcon size={14} /></button>
      </li>)}</ul>
      <div className="row-actions hand-add-steps">
        <button type="button" className="ghost-button" onClick={() => set({ condicoes: { ...r.condicoes, itens: [...r.condicoes.itens, { tipo: 'preco', url: '', produto: '', compara: 'caiu', valor: null }] }, gatilho: g.tipo === 'intervalo' && g.minutos < 15 ? { tipo: 'intervalo', minutos: 60 } : g })}>+ Preço de um produto</button>
        <button type="button" className="ghost-button" onClick={() => set({ condicoes: { ...r.condicoes, itens: [...r.condicoes.itens, { tipo: 'janela', de: '05:00', ate: '12:00' }] } })}>+ Horário</button>
        <button type="button" className="ghost-button" onClick={() => set({ condicoes: { ...r.condicoes, itens: [...r.condicoes.itens, { tipo: 'dia_semana', dias: [0, 1, 2, 3, 4] }] } })}>+ Dias da semana</button>
        <button type="button" className="ghost-button" onClick={() => set({ condicoes: { ...r.condicoes, itens: [...r.condicoes.itens, { tipo: 'programa_aberto', nome: '', aberto: true }] } })}>+ Programa aberto</button>
        <button type="button" className="ghost-button" onClick={() => set({ condicoes: { ...r.condicoes, itens: [...r.condicoes.itens, { tipo: 'arquivo_existe', caminho: '', existe: true }] } })}>+ Arquivo</button>
        <button type="button" className="ghost-button" onClick={() => set({ condicoes: { ...r.condicoes, itens: [...r.condicoes.itens, { tipo: 'uma_vez_por_dia' }] } })}>+ Uma vez por dia</button>
      </div>
      {temPreco && <label className="hand-check"><input type="checkbox" checked={r.nuvem} disabled={!nuvem && !r.nuvem} onChange={e => set({ nuvem: e.target.checked })} />Pode usar a nuvem para achar o preço quando a página não mostrar{nuvem ? ` (pesquisa na web por ${nuvem}; vai só o link ou o nome do produto)` : ' (nenhuma nuvem conectada)'}</label>}

      <h3 className="hand-step-title">Então</h3>
      <ul className="hand-cond-list">{r.acoes.map((a, i) => <li key={i}>
        {a.tipo === 'notificar' && <><span className="hand-verb">Notificar</span><input value={a.titulo} placeholder="Título (opcional)" aria-label="Título" onChange={e => setAcao(i, { ...a, titulo: e.target.value })} />
          <input className="hand-grow" value={a.texto} placeholder={temPreco ? 'Ex.: Caiu para {preco} (antes {anterior})' : 'Ex.: Bom dia! ☀️'} aria-label="Texto" onChange={e => setAcao(i, { ...a, texto: e.target.value })} /></>}
        {a.tipo === 'mensagem_ayla' && <><span className="hand-verb">Mensagem da Ayla</span><input className="hand-grow" value={a.pedido} placeholder="Ex.: um bom dia curto com a minha agenda de hoje" aria-label="O que escrever" onChange={e => setAcao(i, { ...a, pedido: e.target.value })} /></>}
        {a.tipo === 'macro' && <><span className="hand-verb">Macro</span><select aria-label="Macro" value={a.id} onChange={e => setAcao(i, { ...a, id: e.target.value, nome: macros.find(m => m.id === e.target.value)?.nome ?? '' })}><option value="">Escolha…</option>{macros.map(m => <option key={m.id} value={m.id}>{m.nome}</option>)}</select></>}
        {a.tipo === 'comando' && <><span className="hand-verb">Comando</span><input className="hand-command hand-grow" value={a.comando} placeholder="ex.: notify-send oi" aria-label="Comando" spellCheck={false} onChange={e => setAcao(i, { ...a, comando: e.target.value })} /></>}
        <button type="button" className="icon-button" aria-label="Tirar ação" onClick={() => set({ acoes: r.acoes.filter((_, j) => j !== i) })}><TrashIcon size={14} /></button>
      </li>)}</ul>
      <div className="row-actions hand-add-steps">
        <button type="button" className="ghost-button" onClick={() => set({ acoes: [...r.acoes, { tipo: 'notificar', titulo: '', texto: '' }] })}>+ Notificação</button>
        <button type="button" className="ghost-button" onClick={() => set({ acoes: [...r.acoes, { tipo: 'mensagem_ayla', pedido: '' }] })}>+ Mensagem da Ayla</button>
        <button type="button" className="ghost-button" onClick={() => set({ acoes: [...r.acoes, { tipo: 'macro', id: '', nome: '' }] })}>+ Macro</button>
        <button type="button" className="ghost-button" onClick={() => set({ acoes: [...r.acoes, { tipo: 'comando', comando: '', aprovado: '' }] })}>+ Comando</button>
      </div>
      {temPreco && <small className="panel-muted">Na notificação, {'{preco}'} e {'{anterior}'} viram os valores. Toda checagem vai para o histórico (e para o gráfico).</small>}
      {error && <p role="alert" className="app-error">{error}</p>}
      <div className="row-actions">
        {comandos.length
          ? <Confirm className="primary-action" disabled={!r.nome.trim()} question={`Aprovar ${comandos.length === 1 ? 'o comando' : 'os comandos'} ${comandos.map(c => `“${c.comando}”`).join(', ')} para rodar sozinho${comandos.length === 1 ? '' : 's'} nesta rotina?`} confirm="Aprovar e salvar" onConfirm={() => void save()}>Salvar rotina</Confirm>
          : <button className="primary-action" disabled={!r.nome.trim()} onClick={() => void save()}>Salvar rotina</button>}
        <button className="ghost-button" onClick={() => close()}>Cancelar</button>
      </div>
    </section>
  </div>;
}

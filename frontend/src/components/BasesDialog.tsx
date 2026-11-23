import { useEffect, useState } from 'react';
import { errorText, formatBytes, native } from '../services/Desktop';
import { basesChanged, listBases, type Base, type Validation } from '../services/Bases';

/** "+" das bases: baixar do catálogo ou sugerir um link do GitHub, que a Ayla valida e você aprova. */
export default function BasesDialog({ onClose }: { onClose: () => void }) {
  const [catalog, setCatalog] = useState<Base[] | null>(null);
  const [url, setUrl] = useState(''); const [report, setReport] = useState<Validation | null>(null);
  const [busy, setBusy] = useState<string | null>(null); const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const load = () => void listBases().then(setCatalog).catch(e => setError(errorText(e)));
  useEffect(load, []);
  async function run(key: string, task: () => Promise<void>) {
    setBusy(key); setError(''); setNotice('');
    try { await task(); } catch (e) { setError(errorText(e)); } finally { setBusy(null); }
  }
  const install = (base: Base) => run(base.id, async () => { await native('bases_install', { id: base.id }); setNotice(`${base.name} pronta.`); basesChanged(); load(); });
  const validate = () => run('link', async () => { setReport(null); setReport(await native<Validation>('bases_validate', { url })); });
  const approve = (id: string) => run('approve', async () => { await native('bases_approve', { id }); setNotice('Base aprovada e instalada como curada por você.'); setReport(null); setUrl(''); basesChanged(); load(); });
  const discard = (id: string) => { void native('bases_discard', { id }).catch(() => {}); setReport(null); };
  function close() { if (report) discard(report.id); onClose(); }
  return <div className="dialog-backdrop" onClick={() => { if (!busy) close(); }}>
    <div className="dialog bases-dialog" role="dialog" aria-modal="true" aria-labelledby="bases-dialog-title" onClick={e => e.stopPropagation()} onKeyDown={e => { if (e.key === 'Escape' && !busy) close(); }}>
      <h2 id="bases-dialog-title">Bases de dados</h2>
      <p className="row-hint">Dados públicos numa versão fixada e conferida, guardados e consultados neste computador.</p>
      <h3>Catálogo</h3>
      <ul className="bases-catalog">
        {catalog === null ? <li className="row-hint">Carregando…</li> : catalog.map(base => <li key={base.id}>
          <span className="row-text"><span className="row-title">{base.name} <small>· {base.publisher}{base.origin === 'voce' ? ' · adicionada por você' : ''}</small></span>
            <span className="row-hint">{base.installed ? `Instalada · ${base.installed.version ?? base.installed.commit.slice(0, 7)} · ${formatBytes(base.installed.bytes)}` : `${base.description} Download de ${formatBytes(base.download)}.`}</span></span>
          {(!base.installed || !base.current) && <button className="primary-action" disabled={busy !== null} onClick={() => void install(base)}>{busy === base.id ? 'Baixando…' : base.installed ? 'Atualizar' : 'Baixar'}</button>}
        </li>)}
      </ul>
      <h3>Sugerir por link</h3>
      <form className="bases-link" onSubmit={e => { e.preventDefault(); if (url.trim() && !busy) void validate(); }}>
        <input aria-label="Link do repositório" placeholder="https://github.com/orgao/dados (ou /tree/main/pasta)" value={url} onChange={e => { setUrl(e.target.value); if (report) discard(report.id); }} disabled={busy !== null} />
        <button type="submit" disabled={!url.trim() || busy !== null}>{busy === 'link' ? 'Validando…' : 'Validar'}</button>
      </form>
      <p className="row-hint">A Ayla confere licença, formato (migrações SQL ou CSV), tamanho e se a base monta sem erro num commit fixado. Você aprova antes de ela entrar como curada.</p>
      {report && <div className="bases-report" role="region" aria-label="Relatório da validação">
        <p><b>{report.name}</b> · {report.repository} · commit {report.commit.slice(0, 7)}</p>
        <p>{report.format} · licença {report.license || 'não declarada'} · download {formatBytes(report.download)} · base {formatBytes(report.size)} · {report.tables.length} tabelas</p>
        {report.description && <p className="row-hint">{report.description}</p>}
        <ul className="bases-tables">{report.tables.slice(0, 8).map(t => <li key={t}>{t}</li>)}{report.tables.length > 8 && <li>… e mais {report.tables.length - 8}</li>}</ul>
        {report.warnings.map(w => <p key={w} role="alert" className="app-error">{w}</p>)}
        <div className="row-actions"><button className="primary-action" disabled={busy !== null} onClick={() => void approve(report.id)}>{busy === 'approve' ? 'Instalando…' : 'Aprovar e instalar'}</button><button className="ghost-button" disabled={busy !== null} onClick={() => discard(report.id)}>Descartar</button></div>
      </div>}
      {error && <p role="alert" className="app-error">{error}</p>}
      {notice && <p role="status" className="notice">{notice}</p>}
      <div className="row-actions dialog-actions"><button className="ghost-button" disabled={busy !== null} onClick={close}>Fechar</button></div>
    </div>
  </div>;
}

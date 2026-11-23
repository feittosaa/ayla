import { useCallback, useEffect, useState } from 'react';
import Confirm from './Confirm';
import { errorText, formatBytes, native } from '../services/Desktop';
import { basesChanged, listBases, onBasesChanged, type Base } from '../services/Bases';

/** Bases de dados públicas curadas: baixadas de uma versão fixada da fonte oficial, montadas e
 * consultadas neste computador. A Ayla usa a base nas respostas quando a pergunta é sobre ela. */
export default function BasesSettings() {
  const [bases, setBases] = useState<Base[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null); const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const load = useCallback(() => listBases().then(setBases).catch(e => setError(errorText(e))), []);
  useEffect(() => { void load(); return onBasesChanged(() => void load()); }, [load]);
  async function act(base: Base, command: 'bases_install' | 'bases_remove', done: string) {
    setBusy(base.id); setError(''); setNotice('');
    try { await native(command, { id: base.id }); setNotice(done); basesChanged(); } catch (e) { setError(errorText(e)); } finally { setBusy(null); void load(); }
  }
  return <section id="bases" className="settings-group" aria-labelledby="bases-title"><h2 id="bases-title">Bases de dados</h2>
    <div className="group-card">
      {bases === null ? <div className="group-row"><span className="row-hint">Carregando o catálogo…</span></div>
        : bases.map(base => {
          const installed = base.installed;
          const hint = busy === base.id ? (installed ? 'Atualizando…' : 'Baixando e montando…')
            : installed ? `Instalada · ${installed.version ?? installed.commit.slice(0, 7)} · ${formatBytes(installed.bytes)}${base.current ? '' : ' · nova versão disponível'}`
            : `${base.description} Download de ${formatBytes(base.download)}.`;
          return <div key={base.id} className="group-row">
            <span className="row-text">
              <span className="row-title">{base.name} <small>· {base.publisher}{base.origin === 'voce' ? ' · adicionada por você' : ''}</small></span>
              <span className="row-hint">{hint}</span>
              <span className="row-hint">Fonte: <a href={base.source} onClick={e => { e.preventDefault(); void native('open_web', { url: base.source }).catch(err => setError(errorText(err))); }}>{base.repository}</a> · commit {base.commit.slice(0, 7)} · licença {base.license}</span>
            </span>
            <span className="row-actions">
              {(!installed || !base.current) && <button className="primary-action" disabled={busy !== null} onClick={() => void act(base, 'bases_install', installed ? 'Base atualizada.' : 'Base pronta. Pergunte sobre ela na conversa.')}>{installed ? 'Atualizar' : 'Baixar'}</button>}
              {installed && <Confirm className="ghost-button" disabled={busy !== null} question="Remover esta base?" confirm="Remover" onConfirm={() => void act(base, 'bases_remove', 'Base removida. Dá para baixar de novo quando quiser.')}>Remover</Confirm>}
            </span>
          </div>;
        })}
    </div>
    <p className="group-note">Também ficam na barra lateral da conversa, acima de Projetos: clique numa base para usá-la na próxima mensagem; o + adiciona outras, inclusive por link. Dados públicos de fontes oficiais, numa versão fixada e conferida. Ficam neste computador e são consultados sem internet: perguntas sobre eles são respondidas pelo modelo local, citando a base e a versão. Não substituem orientação fiscal oficial.</p>
    {error && <p role="alert" className="app-error">{error}</p>}
    {notice && <p role="status" className="notice">{notice}</p>}
  </section>;
}

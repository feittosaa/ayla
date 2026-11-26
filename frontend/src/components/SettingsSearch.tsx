import { useEffect, useRef, useState } from 'react';

type Match = { section: string; label: string; target: HTMLElement };

function normalize(text: string) {
  return text.normalize('NFD').replace(/\p{M}/gu, '').toLocaleLowerCase('pt-BR');
}

function search(root: HTMLElement, query: string) {
  const terms = normalize(query.trim()).split(/\s+/);
  const matches: Match[] = [];
  for (const group of root.querySelectorAll<HTMLElement>('.settings-group')) {
    const section = group.querySelector('h2')?.textContent ?? '';
    const seen = new Set<string>();
    for (const el of group.querySelectorAll<HTMLElement>('h2, h3, summary, .row-title, label, button, [aria-label], [data-search], .row-hint')) {
      if (el.closest('[role="dialog"], .memory-list') || (group.id === 'lembretes' && el.tagName !== 'H2')) continue;
      const row = el.closest<HTMLElement>('.group-row');
      const target = el.matches('.row-title, .row-hint') && row ? row : el;
      const title = el.querySelector('.row-title') ?? (el.matches('.row-hint') ? row?.querySelector('.row-title') : null);
      const label = (title?.textContent || el.getAttribute('aria-label') || el.textContent || '').trim();
      if (!label || seen.has(label)) continue;
      const text = normalize([section, label, el.dataset.search, el.matches('.row-hint, select, label') ? el.textContent : ''].join(' '));
      if (terms.every(term => text.includes(term))) {
        matches.push({ section, label, target });
        seen.add(label);
      }
    }
  }
  return matches;
}

export default function SettingsSearch() {
  const field = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState('');
  const [matches, setMatches] = useState<Match[]>([]);
  useEffect(() => {
    const root = field.current?.closest<HTMLElement>('.settings-content');
    if (!root || !query.trim()) return;
    const refresh = () => setMatches(search(root, query));
    refresh();
    const observer = new MutationObserver(refresh);
    for (const group of root.querySelectorAll('.settings-group')) {
      observer.observe(group, { subtree: true, childList: true, characterData: true });
    }
    return () => observer.disconnect();
  }, [query]);

  function reveal(target: HTMLElement) {
    const details = target.closest('details');
    if (details) details.open = true;
    setQuery('');
    requestAnimationFrame(() => {
      if (target.tabIndex < 0) target.tabIndex = -1;
      target.focus({ preventScroll: true });
      target.scrollIntoView({ block: 'center' });
    });
  }

  return <div className="settings-search" role="search">
    <input ref={field} type="search" aria-label="Pesquisar configurações" placeholder="Pesquisar configurações" value={query}
      onChange={e => setQuery(e.target.value)} onKeyDown={e => { if (e.key === 'Escape') setQuery(''); }} />
    {query.trim() && <>
      <span role="status">{matches.length ? `${matches.length} resultado${matches.length === 1 ? '' : 's'}` : 'Nenhuma configuração encontrada.'}</span>
      {matches.length > 0 && <ul>{matches.map(({ section, label, target }) => <li key={section + '/' + label}>
        <button type="button" onClick={() => reveal(target)}>{label}{label !== section && <small>{section}</small>}</button>
      </li>)}</ul>}
    </>}
  </div>;
}

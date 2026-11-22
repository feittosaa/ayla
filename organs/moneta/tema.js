/* Camada de compatibilidade da Moneta dentro da Ayla (servida em /__ayla/tema.js, depois de
 * /__ayla/tema.css).
 *
 * No tema "Sistema" da Moneta, claro ou escuro seguem o modo da Ayla: o WebKitGTK não repassa o
 * modo da janela para a página embutida. Quando a aparência da Ayla muda (fundo, modo claro ou
 * escuro), a Ayla manda as novas variáveis por mensagem e elas valem na hora, sem recarregar.
 * Nos temas Claro e Escuro escolhidos na Moneta, ela mostra as próprias cores e nada muda aqui.
 */
(() => {
  const root = document.documentElement;
  const hosts = ['tauri://localhost', 'http://tauri.localhost', 'https://tauri.localhost'];
  let light = root.dataset.aylaMode === 'light';
  const following = () => (globalThis.MonetaTheme?.theme?.() ?? 'system') === 'system';
  function enforce() {
    if (!following()) return;
    const mode = light ? 'light' : 'dark';
    if (root.dataset.theme !== mode) root.dataset.theme = mode;
  }
  new MutationObserver(enforce).observe(root, { attributes: true, attributeFilter: ['data-theme'] });
  addEventListener('moneta-appearance', enforce);
  enforce();
  addEventListener('message', event => {
    const data = event.data;
    if (event.source !== parent || !hosts.includes(event.origin) || data?.type !== 'ayla-tema' || typeof data.vars !== 'object') return;
    for (const [name, value] of Object.entries(data.vars)) {
      if (/^--ayla-[a-z-]+$/.test(name) && typeof value === 'string' && value.length <= 64) root.style.setProperty(name, value);
    }
    light = Boolean(data.light);
    root.dataset.aylaMode = light ? 'light' : 'dark';
    enforce();
  });
})();

/** Configuração do CodeMirror da Athena (bloco de notas): cada opção das Preferências fica num
 * compartimento, para mudar sem recriar o editor nem perder o histórico de desfazer. */
import { copyLineDown, defaultKeymap, deleteLine, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { LanguageDescription, bracketMatching, indentOnInput, indentUnit, syntaxHighlighting, type LanguageSupport } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { gotoLine, highlightSelectionMatches, openSearchPanel, search, searchKeymap } from '@codemirror/search';
import { Compartment, EditorState, type Extension } from '@codemirror/state';
import { EditorView, ViewPlugin, crosshairCursor, drawSelection, highlightActiveLine, highlightActiveLineGutter, highlightSpecialChars, highlightTrailingWhitespace, highlightWhitespace, keymap, lineNumbers, rectangularSelection, type ViewUpdate } from '@codemirror/view';
import { classHighlighter } from '@lezer/highlight';

export type Settings = {
  lineNumbers: boolean; highlightLine: boolean; rightMargin: boolean; marginColumn: number; wrap: boolean;
  showSpaces: boolean; spellcheck: boolean; spaces: boolean; tabWidth: number; autoIndent: boolean; brackets: boolean;
  font: 'mono' | 'system'; fontSize: number; zoom: number; restoreSession: boolean;
};
/** Padrões do Text Editor do GNOME (gsettings org.gnome.TextEditor deste computador). */
export const DEFAULTS: Settings = {
  lineNumbers: false, highlightLine: false, rightMargin: false, marginColumn: 80, wrap: true,
  showSpaces: false, spellcheck: true, spaces: false, tabWidth: 8, autoIndent: true, brackets: true,
  font: 'mono', fontSize: 15, zoom: 1, restoreSession: true,
};

export const languageNames = ['Texto simples', ...languages.map(l => l.name).sort((a, b) => a.localeCompare(b, 'pt-BR'))];

/** Linguagem pelo nome do arquivo (ou pelo nome escolhido na barra de status). */
export function languageFor(file: string | null, chosen?: string | null): LanguageDescription | null {
  if (chosen === 'Texto simples') return null;
  if (chosen) return languages.find(l => l.name === chosen) ?? null;
  return file ? LanguageDescription.matchFilename(languages, file) : null;
}

/** Linha vertical na coluna da margem direita. */
function marginPlugin(column: number) {
  return ViewPlugin.fromClass(class {
    line: HTMLElement;
    constructor(view: EditorView) {
      this.line = document.createElement('div');
      this.line.className = 'pad-margin-line';
      view.scrollDOM.appendChild(this.line);
      this.place(view);
    }
    update(update: ViewUpdate) { if (update.geometryChanged || update.viewportChanged) this.place(update.view); }
    place(view: EditorView) { this.line.style.left = `${view.contentDOM.offsetLeft + column * view.defaultCharacterWidth + 6}px`; }
    destroy() { this.line.remove(); }
  });
}

export const slots = {
  language: new Compartment(), lineNumbers: new Compartment(), highlightLine: new Compartment(), margin: new Compartment(),
  wrap: new Compartment(), spaces: new Compartment(), spell: new Compartment(), indent: new Compartment(),
  autoIndent: new Compartment(), brackets: new Compartment(), readonly: new Compartment(),
};

/** Valores de cada compartimento para as preferências atuais. */
export function configure(settings: Settings, language: LanguageSupport | null, readonly: boolean): [Compartment, Extension][] {
  const unit = settings.spaces ? ' '.repeat(settings.tabWidth) : '\t';
  return [
    [slots.language, language ?? []],
    [slots.lineNumbers, settings.lineNumbers ? [lineNumbers(), highlightActiveLineGutter()] : []],
    [slots.highlightLine, settings.highlightLine ? highlightActiveLine() : []],
    [slots.margin, settings.rightMargin ? marginPlugin(settings.marginColumn) : []],
    [slots.wrap, settings.wrap ? EditorView.lineWrapping : []],
    [slots.spaces, settings.showSpaces ? [highlightWhitespace(), highlightTrailingWhitespace()] : []],
    [slots.spell, EditorView.contentAttributes.of({ spellcheck: String(settings.spellcheck), 'aria-label': 'Texto do documento' })],
    [slots.indent, [indentUnit.of(unit), EditorState.tabSize.of(settings.tabWidth)]],
    [slots.autoIndent, settings.autoIndent ? indentOnInput() : []],
    [slots.brackets, settings.brackets ? bracketMatching() : []],
    [slots.readonly, EditorState.readOnly.of(readonly)],
  ];
}

/** Atalhos do Text Editor que o CodeMirror não tem com as mesmas teclas. */
const textEditorKeys = keymap.of([
  { key: 'Mod-i', run: gotoLine, preventDefault: true },
  { key: 'Mod-h', run: openSearchPanel, preventDefault: true },
  { key: 'Mod-d', run: deleteLine, preventDefault: true },
  { key: 'Mod-Shift-d', run: copyLineDown, preventDefault: true },
]);

/** Painel de busca e "ir para linha" em português. */
const phrases = EditorState.phrases.of({
  'Go to line': 'Ir para a linha', 'go': 'ir', 'Find': 'Localizar', 'Replace': 'Substituir', 'next': 'próximo',
  'previous': 'anterior', 'all': 'todos', 'match case': 'diferenciar maiúsculas', 'regexp': 'expressão regular',
  'by word': 'palavra inteira', 'replace': 'substituir', 'replace all': 'substituir todos', 'close': 'fechar',
  'current match': 'ocorrência atual', 'replaced $ matches': '$ ocorrências substituídas',
  'replaced match on line $': 'ocorrência substituída na linha $', 'on line': 'na linha',
});

export function extensions(settings: Settings, language: LanguageSupport | null, readonly: boolean, onUpdate: (update: ViewUpdate) => void): Extension[] {
  return [
    ...configure(settings, language, readonly).map(([slot, value]) => slot.of(value)),
    history(), drawSelection(), highlightSpecialChars(), rectangularSelection(), crosshairCursor(),
    highlightSelectionMatches(), search({ top: true }), syntaxHighlighting(classHighlighter),
    phrases, textEditorKeys, keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
    EditorView.updateListener.of(onUpdate),
  ];
}

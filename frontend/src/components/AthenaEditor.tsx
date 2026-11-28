import { useLayoutEffect, useRef } from 'react';
import { closeBrackets, closeBracketsKeymap, insertBracket } from '@codemirror/autocomplete';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { syntaxTree } from '@codemirror/language';
import { Annotation, Compartment, EditorState, Prec, StateEffect, StateField, Transaction, type Range } from '@codemirror/state';
import type { SyntaxNodeRef } from '@lezer/common';
import { createRoot, type Root } from 'react-dom/client';
import Markdown from './Markdown';
import Diagram from './Diagram';
import { Decoration, EditorView, keymap, placeholder, ViewPlugin, WidgetType, type ViewUpdate } from '@codemirror/view';
import { IMAGE, imageWidth, isAttachment, mediaVersion, resizedImage } from '../services/Wiki';
import { mediaUrl } from '../services/Media';

type Atom = { label: string; target?: string; image?: boolean; width?: number; title?: string; version?: number };
type Props = {
  value: string;
  root: string;
  readOnly: boolean;
  autoFocus: boolean;
  editor: { current: EditorView | null };
  onChange: (text: string, caret: number) => void;
  onPaste: (event: ClipboardEvent, view: EditorView) => boolean;
  onBlur: () => void;
  onEscape: () => void;
  /** Teclas da lista de sugestões de link; devolve true quando a lista usou a tecla. */
  onSuggestKey: (key: 'Enter' | 'Tab' | 'ArrowUp' | 'ArrowDown') => boolean;
  onLink: (target: string) => void;
  /** O cursor mudou de lugar sem digitar (clique, setas). */
  onCaret?: (caret: number) => void;
  /** O editor ficou pronto (para voltar ao cursor e à tela de antes). */
  onReady?: (view: EditorView) => void;
  /** `![[nome]]` sem extensão: o anexo com esse nome, se houver. */
  embed?: (name: string) => string | undefined;
  /** Muda quando os anexos da raiz mudam (imagem trocada no lugar): redesenha as imagens. */
  media?: unknown;
  /** Busca na nota (Ctrl+F): ocorrências [início, fim) no texto do editor e qual é a atual; `step`
   * muda a cada navegação, para levar a atual à tela. */
  find?: { matches: [number, number][]; current: number; step: number } | null;
};

const external = Annotation.define<boolean>();
/** Soltou o mouse: as marcas voltam a seguir o cursor. */
const pointerUp = StateEffect.define<null>();
/** Os anexos mudaram: refaz as marcas (imagens com o endereço novo). */
const mediaChanged = StateEffect.define<null>();
/** Busca na nota: destaca as ocorrências (a atual com outra cor). */
const findChanged = StateEffect.define<{ matches: [number, number][]; current: number } | null>();
const findMark = Decoration.mark({ class: 'athena-find' });
const findCurrent = Decoration.mark({ class: 'athena-find athena-find-current' });
const findField = StateField.define({
  create: () => Decoration.none,
  update(value, tr) {
    for (const effect of tr.effects) if (effect.is(findChanged)) {
      const found = effect.value; if (!found) return Decoration.none;
      const size = tr.state.doc.length;
      return Decoration.set(found.matches.filter(([from, to]) => from < to && to <= size)
        .map(([from, to], i) => (i === found.current ? findCurrent : findMark).range(from, to)), true);
    }
    return tr.docChanged ? value.map(tr.changes) : value;
  },
  provide: field => EditorView.decorations.from(field),
});
const wikiPattern = /^(!?)\[\[([^\]\n|]+)(?:\|([^\]\n]+))?\]\]/;
// Pares fechados sozinhos, como no Obsidian: digitar o fechamento ou dar Enter logo antes dele
// sai do par; Backspace no par vazio apaga os dois.
const pairs = { brackets: ['[', '{', '*'] };
/** Enter logo antes de fechamentos que a própria Athena colocou (ex.: `]]`): pula todos eles. */
function leavePair(view: EditorView) {
  if (!view.state.selection.main.empty || view.state.selection.ranges.length > 1) return false;
  const skips: Transaction[] = [];
  for (let state = view.state; ;) {
    const head = state.selection.main.head;
    const next = state.sliceDoc(head, head + 1);
    const skip = next ? insertBracket(state, next) : null;
    if (!skip || !skip.newDoc.eq(state.doc)) break;
    skips.push(skip); state = skip.state;
  }
  if (!skips.length) return false;
  view.dispatch(...skips);
  return true;
}
const localTarget = (value: string) => {
  try { return decodeURIComponent(value); }
  catch { return value; }
};
// Markdown com GFM (como o modo Ver e o Obsidian): endereços soltos (https://… e www.…) viram
// links clicáveis, além de tachado e caixas de tarefa.
const language = markdown({
  base: markdownLanguage,
  extensions: {
    defineNodes: ['AthenaLink'],
    parseInline: [{
      name: 'AthenaLink',
      before: 'Link',
      parse(context, next, position) {
        if (next !== 33 && next !== 91) return -1;
        const raw = context.slice(position, context.end).match(wikiPattern)?.[0];
        return raw ? context.addElement(context.elt('AthenaLink', position, position + raw.length)) : -1;
      },
    }],
  },
});

class InlineAtom extends WidgetType {
  constructor(
    readonly atom: Atom,
    readonly from: number,
    readonly to: number,
    readonly options: { current: Props },
  ) { super(); }

  eq(other: InlineAtom) {
    return this.from === other.from && this.to === other.to && this.atom.target === other.atom.target
      && this.atom.label === other.atom.label && this.atom.width === other.atom.width
      && this.atom.title === other.atom.title && this.atom.image === other.atom.image && this.atom.version === other.atom.version;
  }

  toDOM(view: EditorView) {
    const { label, target, image, width, title } = this.atom;
    if (image && target && !/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(target)) {
      const frame = document.createElement('span');
      frame.className = 'note-resizable-image';
      const picture = document.createElement('img');
      picture.className = 'note-image';
      picture.alt = label;
      picture.src = mediaUrl({ root: this.options.current.root, path: target.split('#')[0] });
      picture.draggable = false;
      if (width) picture.width = width;
      picture.addEventListener('load', () => view.requestMeasure());
      frame.append(picture);
      for (const side of [-1, 1]) {
        const handle = document.createElement('button');
        handle.type = 'button';
        handle.className = 'note-resize-handle';
        handle.dataset.side = String(side);
        handle.setAttribute('aria-label', `Redimensionar ${label} pela borda ${side < 0 ? 'esquerda' : 'direita'}`);
        handle.title = 'Arraste para ajustar a largura';
        const clamp = (value: number) => Math.min(view.contentDOM.clientWidth, Math.max(24, Math.round(value)));
        const original = view.state.doc.sliceString(this.from, this.to);
        const save = (next: number) => {
          if (view.state.readOnly || view.state.doc.sliceString(this.from, this.to) !== original) return;
          view.dispatch({ changes: { from: this.from, to: this.to, insert: resizedImage(original, target, label, title, next) } });
          view.focus();
        };
        let drag: { x: number; width: number } | null = null;
        const reset = () => {
          drag = null;
          if (width) picture.width = width;
          else picture.removeAttribute('width');
          view.requestMeasure();
        };
        handle.onpointerdown = event => {
          if (event.button !== 0 || view.state.readOnly) return;
          event.preventDefault();
          handle.setPointerCapture(event.pointerId);
          drag = { x: event.clientX, width: picture.getBoundingClientRect().width };
        };
        handle.onpointermove = event => {
          if (!drag) return;
          picture.width = clamp(drag.width + side * (event.clientX - drag.x));
          view.requestMeasure();
        };
        handle.onpointerup = event => {
          const start = drag;
          reset();
          if (start && event.clientX !== start.x) save(clamp(start.width + side * (event.clientX - start.x)));
        };
        handle.onlostpointercapture = reset;
        handle.onkeydown = event => {
          if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
          event.preventDefault();
          save(clamp(picture.getBoundingClientRect().width + side * (event.key === 'ArrowRight' ? 10 : -10)));
        };
        frame.append(handle);
      }
      picture.addEventListener('error', () => { frame.textContent = `[imagem não encontrada: ${label}]`; view.requestMeasure(); });
      return frame;
    }

    if (target && !image) {
      const anchor = document.createElement('a');
      anchor.className = 'note-link';
      anchor.href = /^(https?:\/\/|\/\/)/i.test(target) ? target : '#';
      anchor.title = target;
      anchor.textContent = label;
      anchor.onclick = event => {
        event.preventDefault();
        this.options.current.onLink(target);
      };
      anchor.onauxclick = event => {
        if (event.button === 1) { event.preventDefault(); this.options.current.onLink(target); }
      };
      return anchor;
    }
    const span = document.createElement('span');
    span.textContent = label;
    return span;
  }
}

/** Bloco ```mermaid: o código do diagrama, ou `null` se for outro bloco de código. */
function diagramCode(state: EditorState, node: SyntaxNodeRef) {
  if (node.name !== 'FencedCode') return null;
  const info = node.node.getChild('CodeInfo');
  if (!info || state.sliceDoc(info.from, info.to).trim().toLowerCase() !== 'mermaid') return null;
  const text = node.node.getChild('CodeText');
  return text ? state.sliceDoc(text.from, text.to) : '';
}
// Tabela e diagrama só voltam ao texto com o cursor parado dentro: uma seleção que passa por eles
// (arrastando o mouse) não os desmonta, senão o texto pula debaixo do ponteiro e a seleção se perde.
const touches = (state: EditorState, from: number, to: number) => state.selection.ranges.some(r => r.empty && r.head >= from && r.head <= to);
/** Tabela ou diagrama que aparece desenhado: o cursor não está nele (como no Obsidian). */
function drawnBlock(state: EditorState, node: SyntaxNodeRef) {
  if (node.name !== 'Table' && diagramCode(state, node) === null) return null;
  const from = state.doc.lineAt(node.from).from; const to = state.doc.lineAt(node.to).to;
  return touches(state, from, to) ? null : { from, to };
}

const roots = new WeakMap<HTMLElement, Root>();
class DrawnBlock extends WidgetType {
  constructor(readonly kind: 'table' | 'diagram', readonly text: string, readonly options: { current: Props }) { super(); }
  eq(other: DrawnBlock) { return other.kind === this.kind && other.text === this.text; }
  toDOM(view: EditorView) {
    const dom = document.createElement('div');
    dom.className = `athena-drawn athena-drawn-${this.kind}`;
    dom.title = 'Clique para editar';
    const root = createRoot(dom);
    root.render(this.kind === 'table'
      ? <Markdown text={this.text} onNoteLink={target => this.options.current.onLink(target)} />
      : <Diagram code={this.text} />);
    roots.set(dom, root);
    // Clique fora de um link: volta ao texto do bloco para editar.
    dom.addEventListener('mousedown', event => {
      if (event.button !== 0 || (event.target as HTMLElement).closest('a') || view.state.readOnly) return;
      event.preventDefault();
      view.dispatch({ selection: { anchor: view.posAtDOM(dom) } });
      view.focus();
    });
    return dom;
  }
  destroy(dom: HTMLElement) { const root = roots.get(dom); if (root) queueMicrotask(() => root.unmount()); }
  ignoreEvent() { return true; }
  get estimatedHeight() { return this.kind === 'diagram' ? 320 : 36 * (this.text.split('\n').length - 1); }
}

function drawnBlocks(state: EditorState, options: { current: Props }) {
  const ranges: Range<Decoration>[] = [];
  syntaxTree(state).iterate({
    enter(node) {
      const block = drawnBlock(state, node);
      if (!block) return node.name !== 'Table' && node.name !== 'FencedCode';
      const code = diagramCode(state, node);
      const widget = new DrawnBlock(code === null ? 'table' : 'diagram', code ?? state.sliceDoc(block.from, block.to), options);
      ranges.push(Decoration.replace({ widget, block: true }).range(block.from, block.to));
      return false;
    },
  });
  return Decoration.set(ranges, true);
}

function decorations(view: EditorView, options: { current: Props }) {
  const ranges: Range<Decoration>[] = [];
  const hidden: Range<Decoration>[] = [];
  const tree = syntaxTree(view.state);
  const slice = (from: number, to: number) => view.state.doc.sliceString(from, to);
  const atom = (from: number, to: number, data: Atom) => {
    hidden.push(Decoration.replace({ widget: new InlineAtom(data, from, to, options) }).range(from, to));
  };
  const hide = (from: number, to: number) => hidden.push(Decoration.replace({}).range(from, to));
  // Link com o cursor dentro ou encostado aparece como texto cru (como no Obsidian): as setas
  // chegam à borda, o link abre e o cursor entra nos colchetes. Imagens só abrem com o cursor dentro.
  const editing = (from: number, to: number, edge = true) => view.state.selection.ranges.some(r => edge ? r.head >= from && r.head <= to : r.head > from && r.head < to);
  // Marcas de formatação (#, *, ~~, `, >) aparecem enquanto o cursor toca o trecho, como no Obsidian.
  // Só a ponta da seleção (onde está o cursor) conta, como no Obsidian: ao arrastar o mouse por
  // vários trechos, eles não abrem todos ao mesmo tempo empurrando o texto.
  const touched = (from: number, to: number) => view.state.selection.ranges.some(r => r.head >= from && r.head <= to);
  const styles: Record<string, string> = {
    StrongEmphasis: 'athena-strong', Emphasis: 'athena-emphasis', Strikethrough: 'athena-strike',
    InlineCode: 'athena-code', CodeText: 'athena-code', Blockquote: 'athena-quote',
  };
  const { from, to } = view.viewport;
  tree.iterate({
    from, to,
    enter({ name, node, from, to }) {
      const line = view.state.doc.lineAt(from);
      // Desenhado inteiro pelo campo dos blocos (tabela, diagrama).
      if (drawnBlock(view.state, node)) return false;
      if (name === 'AthenaLink') {
        const [, bang, target, alias] = wikiPattern.exec(slice(from, to))!;
        if (editing(from, to, !bang)) return false;
        const width = imageWidth(alias);
        const clean = (bang && options.current.embed?.(target.trim().split('#')[0])) || target.trim();
        const path = clean.split('#')[0];
        const image = Boolean(bang && IMAGE.test(path));
        atom(from, to, { target: isAttachment(path) ? path : clean, label: width ? target.trim() : alias || target.trim(), image, width: Number(width?.split('x')[0]) || undefined, version: image ? mediaVersion(options.current.root, path) : undefined });
        return false;
      }
      if ((name === 'Link' || name === 'Image' || name === 'Autolink') && to <= line.to) {
        const url = node.getChild('URL');
        if (!url || editing(from, to, name !== 'Image')) return;
        const marks = node.getChildren('LinkMark');
        const destination = slice(url.from, url.to).replace(/^<|>$/g, '');
        const label = name === 'Autolink' ? destination : slice(marks[0].to, marks[1].from);
        const title = node.getChild('LinkTitle');
        const titleText = title ? slice(title.from + 1, title.to - 1) : undefined;
        const target = /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(destination) ? destination : localTarget(destination);
        atom(from, to, { target, label: label || target, title: titleText, image: name === 'Image', width: Number(imageWidth(titleText)?.split('x')[0]) || undefined, version: name === 'Image' ? mediaVersion(options.current.root, target.split('#')[0]) : undefined });
        return false;
      }
      if (name === 'URL' && /^(https?:\/\/|www\.)/i.test(slice(from, to)) && !editing(from, to)) {
        const url = slice(from, to);
        atom(from, to, { label: url, target: /^www\./i.test(url) ? `https://${url}` : url });
        return false;
      }
      // `---` vira uma linha; com o cursor nela, volta a ser texto (como no Obsidian).
      if (name === 'HorizontalRule') {
        if (view.state.selection.ranges.some(r => r.head >= line.from && r.head <= line.to)) return false;
        ranges.push(Decoration.line({ class: 'athena-rule' }).range(line.from));
        hide(from, to);
        return false;
      }
      if (name.startsWith('ATXHeading') || name.startsWith('SetextHeading')) {
        ranges.push(Decoration.line({ class: `athena-heading athena-heading-${name.slice(-1)}` }).range(line.from));
      }
      if (styles[name]) ranges.push(Decoration.mark({ class: styles[name] }).range(from, to));
      if (['EmphasisMark', 'StrikethroughMark', 'CodeMark', 'CodeInfo'].includes(name) && !touched(node.parent?.from ?? from, node.parent?.to ?? to)) hide(from, to);
      if ((name === 'HeaderMark' && !touched(node.parent?.from ?? line.from, node.parent?.to ?? line.to))
        || (name === 'QuoteMark' && !touched(line.from, line.to))) {
        const space = slice(to, line.to).match(/^[ \t]+/)?.[0].length ?? 0;
        hide(from, to + space);
      }
      if (name === 'ListMark' && /^[-+*]$/.test(slice(from, to))) atom(from, to, { label: '•' });
      if (name === 'TaskMarker') atom(from, to, { label: /x/i.test(slice(from, to)) ? '☑' : '☐' });
    },
  });
  return { visible: Decoration.set([...ranges, ...hidden], true), atomic: Decoration.set(hidden, true) };
}

export default function AthenaEditor(props: Props) {
  const mount = useRef<HTMLDivElement>(null);
  const options = useRef(props);
  const readOnly = useRef(new Compartment());
  useLayoutEffect(() => { options.current = props; });
  useLayoutEffect(() => {
    const initial = options.current;
    // Enquanto o mouse seleciona, as marcas não abrem nem fecham: o texto pularia debaixo do
    // ponteiro e a seleção piscaria. Ao soltar, voltam a seguir o cursor.
    let selecting = false;
    const plugin = ViewPlugin.fromClass(class {
      decorations: ReturnType<typeof decorations>;
      constructor(view: EditorView) { this.decorations = decorations(view, options); }
      update(update: ViewUpdate) {
        const released = update.transactions.some(t => t.effects.some(e => e.is(pointerUp) || e.is(mediaChanged)));
        const onlySelection = !update.docChanged && !update.viewportChanged && syntaxTree(update.state) === syntaxTree(update.startState);
        if (selecting && onlySelection && !released) return;
        if (released || update.docChanged || update.viewportChanged || update.selectionSet || syntaxTree(update.state) !== syntaxTree(update.startState)) {
          this.decorations = decorations(update.view, options);
        }
      }
    }, {
      decorations: value => value.decorations.visible,
      provide: extension => EditorView.atomicRanges.of(view => view.plugin(extension)?.decorations.atomic ?? Decoration.none),
    });
    const drawn = StateField.define({
      create: state => drawnBlocks(state, options),
      update: (value, tr) => tr.docChanged || tr.selection || syntaxTree(tr.state) !== syntaxTree(tr.startState) ? drawnBlocks(tr.state, options) : value,
      provide: field => EditorView.decorations.from(field),
    });
    const view = new EditorView({
      parent: mount.current!,
      doc: initial.value,
      extensions: [
        language, language.language.data.of({ closeBrackets: pairs }), closeBrackets(), plugin, drawn, history(), findField,
        Prec.highest(keymap.of((['Enter', 'Tab', 'ArrowUp', 'ArrowDown'] as const).map(key => ({ key, run: () => options.current.onSuggestKey(key) })))),
        Prec.high(keymap.of([{ key: 'Enter', run: leavePair }])),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap]),
        EditorView.lineWrapping, placeholder('Comece a escrever…'),
        EditorView.contentAttributes.of({ 'aria-label': 'Conteúdo da nota', spellcheck: 'true' }),
        EditorView.inputHandler.of((view, from, to, text) => {
          if (view.state.readOnly || view.composing) return false;
          // ">" com texto selecionado vira citação nas linhas selecionadas (como no Obsidian), sem apagar o texto.
          if (text === '>' && view.state.selection.ranges.some(r => !r.empty)) {
            const lines = new Set<number>();
            for (const r of view.state.selection.ranges) {
              if (r.empty) continue;
              const first = view.state.doc.lineAt(r.from).number;
              const end = view.state.doc.lineAt(r.to);
              const last = end.from === r.to && end.number > first ? end.number - 1 : end.number;
              for (let n = first; n <= last; n++) lines.add(n);
            }
            const changes = [...lines].sort((a, b) => a - b).map(n => { const line = view.state.doc.line(n); return { from: line.from, insert: line.text.trim() ? '> ' : '>' }; });
            view.dispatch(view.state.update({ changes, userEvent: 'input.type' }));
            return true;
          }
          if (text !== '-' || from !== to) return false;
          const line = view.state.doc.lineAt(from);
          if (line.number === 1 || line.length || !view.state.doc.line(line.number - 1).text.trim()) return false;
          for (let node = syntaxTree(view.state).resolveInner(from, -1); node.parent; node = node.parent) {
            if (node.name === 'FencedCode' || node.name === 'CodeBlock') return false;
          }
          view.dispatch({ changes: { from, insert: '\n-' }, selection: { anchor: from + 2 }, userEvent: 'input.type' });
          return true;
        }),
        readOnly.current.of(EditorState.readOnly.of(initial.readOnly)),
        EditorView.domEventHandlers({
          mousedown: event => { if (event.button === 0) selecting = true; return false; },
          paste: (event, view) => options.current.onPaste(event, view),
          blur: () => options.current.onBlur(),
          keydown: event => { if (event.key === 'Escape') options.current.onEscape(); },
        }),
        EditorView.updateListener.of(update => {
          if (update.docChanged && !update.transactions.some(transaction => transaction.annotation(external))) {
            options.current.onChange(update.state.doc.toString(), update.state.selection.main.head);
          } else if (update.selectionSet && !update.docChanged) options.current.onCaret?.(update.state.selection.main.head);
        }),
      ],
    });
    initial.editor.current = view;
    if (initial.autoFocus) view.focus();
    initial.onReady?.(view);
    const release = () => { if (selecting) { selecting = false; view.dispatch({ effects: pointerUp.of(null) }); } };
    window.addEventListener('mouseup', release);
    return () => { window.removeEventListener('mouseup', release); initial.editor.current = null; view.destroy(); };
  }, []);
  useLayoutEffect(() => {
    const view = props.editor.current;
    if (!view) return;
    const value = props.value.replace(/\r\n/g, '\n');
    if (view.state.doc.toString() !== value) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: value },
        selection: { anchor: Math.min(value.length, view.state.selection.main.head) },
        annotations: [external.of(true), Transaction.addToHistory.of(false)],
      });
    }
    if (view.state.readOnly !== props.readOnly) view.dispatch({ effects: readOnly.current.reconfigure(EditorState.readOnly.of(props.readOnly)) });
  }, [props.editor, props.value, props.readOnly]);
  useLayoutEffect(() => { props.editor.current?.dispatch({ effects: mediaChanged.of(null) }); }, [props.editor, props.media]);
  const find = props.find;
  useLayoutEffect(() => { props.editor.current?.dispatch({ effects: findChanged.of(find ? { matches: find.matches, current: find.current } : null) }); }, [props.editor, find]);
  // Leva a ocorrência atual à tela (selecionada), sem tirar o foco da caixa de busca.
  const step = find?.step; const target = find?.matches[find.current];
  useLayoutEffect(() => {
    const view = props.editor.current;
    if (!view || !target || target[1] > view.state.doc.length) return;
    view.dispatch({ selection: { anchor: target[0], head: target[1] }, effects: EditorView.scrollIntoView(target[0], { y: 'center' }) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.editor, step]);
  return <div ref={mount} className="athena-visual-editor markdown" />;
}

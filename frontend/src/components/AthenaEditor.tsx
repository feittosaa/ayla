import { useLayoutEffect, useRef } from 'react';
import { closeBrackets, closeBracketsKeymap, insertBracket } from '@codemirror/autocomplete';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { syntaxTree } from '@codemirror/language';
import { Annotation, Compartment, EditorState, Prec, Transaction, type Range } from '@codemirror/state';
import { Decoration, EditorView, keymap, placeholder, ViewPlugin, WidgetType, type ViewUpdate } from '@codemirror/view';
import { IMAGE, imageWidth, isAttachment, resizedImage } from '../services/Wiki';
import { mediaUrl } from '../services/Media';

type Atom = { label: string; target?: string; image?: boolean; width?: number; title?: string };
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
};

const external = Annotation.define<boolean>();
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
      && this.atom.title === other.atom.title && this.atom.image === other.atom.image;
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

function decorations(view: EditorView, options: { current: Props }) {
  const ranges: Range<Decoration>[] = [];
  const hidden: Range<Decoration>[] = [];
  const tree = syntaxTree(view.state);
  const slice = (from: number, to: number) => view.state.doc.sliceString(from, to);
  const atom = (from: number, to: number, data: Atom) => {
    hidden.push(Decoration.replace({ widget: new InlineAtom(data, from, to, options) }).range(from, to));
  };
  const hide = (from: number, to: number) => hidden.push(Decoration.replace({}).range(from, to));
  // Link com o cursor dentro aparece como texto cru (como no Obsidian): virar bloco enquanto se
  // digita empurraria o cursor para depois do fechamento.
  const editing = (from: number, to: number) => view.state.selection.ranges.some(r => r.head > from && r.head < to);
  const styles: Record<string, string> = {
    StrongEmphasis: 'athena-strong', Emphasis: 'athena-emphasis', Strikethrough: 'athena-strike',
    InlineCode: 'athena-code', CodeText: 'athena-code', Blockquote: 'athena-quote',
  };
  const { from, to } = view.viewport;
  tree.iterate({
    from, to,
    enter({ name, node, from, to }) {
      const line = view.state.doc.lineAt(from);
      if (name === 'AthenaLink') {
        if (editing(from, to)) return false;
        const [, bang, target, alias] = wikiPattern.exec(slice(from, to))!;
        const clean = target.trim();
        const width = imageWidth(alias);
        const path = clean.split('#')[0];
        const image = Boolean(bang && IMAGE.test(path));
        atom(from, to, { target: isAttachment(path) ? path : clean, label: width ? clean : alias || clean, image, width: Number(width?.split('x')[0]) || undefined });
        return false;
      }
      if ((name === 'Link' || name === 'Image' || name === 'Autolink') && to <= line.to) {
        const url = node.getChild('URL');
        if (!url || editing(from, to)) return;
        const marks = node.getChildren('LinkMark');
        const destination = slice(url.from, url.to).replace(/^<|>$/g, '');
        const label = name === 'Autolink' ? destination : slice(marks[0].to, marks[1].from);
        const title = node.getChild('LinkTitle');
        const titleText = title ? slice(title.from + 1, title.to - 1) : undefined;
        const target = /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(destination) ? destination : localTarget(destination);
        atom(from, to, { target, label: label || target, title: titleText, image: name === 'Image', width: Number(imageWidth(titleText)?.split('x')[0]) || undefined });
        return false;
      }
      if (name === 'URL' && /^(https?:\/\/|www\.)/i.test(slice(from, to)) && !editing(from, to)) {
        const url = slice(from, to);
        atom(from, to, { label: url, target: /^www\./i.test(url) ? `https://${url}` : url });
        return false;
      }
      if (name.startsWith('ATXHeading') || name.startsWith('SetextHeading')) {
        ranges.push(Decoration.line({ class: `athena-heading athena-heading-${name.slice(-1)}` }).range(line.from));
      }
      if (styles[name]) ranges.push(Decoration.mark({ class: styles[name] }).range(from, to));
      if (['EmphasisMark', 'StrikethroughMark', 'CodeMark', 'CodeInfo'].includes(name)) hide(from, to);
      if (name === 'HeaderMark' || name === 'QuoteMark') {
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
    const plugin = ViewPlugin.fromClass(class {
      decorations: ReturnType<typeof decorations>;
      constructor(view: EditorView) { this.decorations = decorations(view, options); }
      update(update: ViewUpdate) {
        if (update.docChanged || update.viewportChanged || update.selectionSet || syntaxTree(update.state) !== syntaxTree(update.startState)) {
          this.decorations = decorations(update.view, options);
        }
      }
    }, {
      decorations: value => value.decorations.visible,
      provide: extension => EditorView.atomicRanges.of(view => view.plugin(extension)?.decorations.atomic ?? Decoration.none),
    });
    const view = new EditorView({
      parent: mount.current!,
      doc: initial.value,
      extensions: [
        language, language.language.data.of({ closeBrackets: pairs }), closeBrackets(), plugin, history(),
        Prec.highest(keymap.of((['Enter', 'Tab', 'ArrowUp', 'ArrowDown'] as const).map(key => ({ key, run: () => options.current.onSuggestKey(key) })))),
        Prec.high(keymap.of([{ key: 'Enter', run: leavePair }])),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap]),
        EditorView.lineWrapping, placeholder('Comece a escrever…'),
        EditorView.contentAttributes.of({ 'aria-label': 'Conteúdo da nota', spellcheck: 'true' }),
        EditorView.inputHandler.of((view, from, to, text) => {
          if (view.state.readOnly || view.composing) return false;
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
          paste: (event, view) => options.current.onPaste(event, view),
          blur: () => options.current.onBlur(),
          keydown: event => { if (event.key === 'Escape') options.current.onEscape(); },
        }),
        EditorView.updateListener.of(update => {
          if (update.docChanged && !update.transactions.some(transaction => transaction.annotation(external))) {
            options.current.onChange(update.state.doc.toString(), update.state.selection.main.head);
          }
        }),
      ],
    });
    initial.editor.current = view;
    if (initial.autoFocus) view.focus();
    return () => { initial.editor.current = null; view.destroy(); };
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
  return <div ref={mount} className="athena-visual-editor markdown" />;
}

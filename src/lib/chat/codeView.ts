// The read-only code editor (CodeMirror 6) the files panel shows files in,
// and the marks a live edit draws in it: the text being replaced (red, struck
// through), the text typed so far (green) and a blinking caret where the next
// character goes, plus a small green "+" left of every added line. Colors
// come from the app's theme variables.
import { Compartment, EditorState, RangeSet, StateEffect, StateField, type Extension, type Range } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  GutterMarker,
  WidgetType,
  gutter,
  lineNumbers,
  type DecorationSet,
} from "@codemirror/view";
import { HighlightStyle, StreamLanguage, syntaxHighlighting, type StreamParser } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";

export interface Span {
  from: number;
  to: number;
}

export interface LiveMarks {
  /** The lines being changed (a bar in the margin). */
  zone?: Span | null;
  /** Lines going away. */
  del: Span[];
  /** Text typed so far. */
  add: Span[];
  /** Where the next character goes. */
  caret: number | null;
  /** The change has landed: the green fades out. */
  settle?: boolean;
}

export const setLiveMarks = StateEffect.define<LiveMarks | null>();

class CaretWidget extends WidgetType {
  toDOM() {
    const el = document.createElement("span");
    el.className = "cm-live-caret";
    el.textContent = "▍";
    return el;
  }
  eq() {
    return true;
  }
}

function buildMarks(state: EditorState, marks: LiveMarks | null): DecorationSet {
  if (!marks) return Decoration.none;
  const doc = state.doc;
  const out: Range<Decoration>[] = [];
  const lines = (from: number, to: number, cls: string) => {
    const first = doc.lineAt(from).number;
    // A range that ends just after a newline does not reach the next line.
    const last = doc.lineAt(Math.max(from, to > from ? to - 1 : to)).number;
    for (let n = first; n <= last; n++) out.push(Decoration.line({ class: cls }).range(doc.line(n).from));
  };
  const clamp = (p: number) => Math.max(0, Math.min(p, doc.length));
  if (marks.zone && marks.zone.to >= marks.zone.from) {
    lines(clamp(marks.zone.from), clamp(marks.zone.to), "cm-live-zone");
  }
  for (const d of marks.del) {
    const from = clamp(d.from);
    const to = clamp(d.to);
    if (to <= from) continue;
    lines(from, to, "cm-live-del-line");
    out.push(Decoration.mark({ class: "cm-live-del" }).range(from, to));
  }
  const settle = marks.settle ? " cm-live-settle" : "";
  for (const a of marks.add) {
    const from = clamp(a.from);
    const to = clamp(a.to);
    if (to <= from) continue;
    lines(from, to, "cm-live-add-line" + settle);
    out.push(Decoration.mark({ class: "cm-live-add" + settle }).range(from, to));
  }
  if (marks.caret != null) {
    out.push(Decoration.widget({ widget: new CaretWidget(), side: 1 }).range(clamp(marks.caret)));
  }
  return Decoration.set(out, true);
}

const liveField = StateField.define<{ marks: LiveMarks | null; deco: DecorationSet }>({
  create: () => ({ marks: null, deco: Decoration.none }),
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setLiveMarks)) return { marks: e.value, deco: buildMarks(tr.state, e.value) };
    if (tr.docChanged && value.marks) {
      const map = (p: number, assoc: number) => tr.changes.mapPos(p, assoc);
      const m = value.marks;
      const marks: LiveMarks = {
        ...m,
        zone: m.zone && { from: map(m.zone.from, -1), to: map(m.zone.to, 1) },
        del: m.del.map((d) => ({ from: map(d.from, 1), to: map(d.to, -1) })),
        add: m.add.map((a) => ({ from: map(a.from, -1), to: map(a.to, 1) })),
        caret: m.caret == null ? null : map(m.caret, 1),
      };
      return { marks, deco: buildMarks(tr.state, marks) };
    }
    return value;
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.deco),
});

// ---- added lines: a small green "+" at the far left ----
class PlusMarker extends GutterMarker {
  toDOM() {
    return document.createTextNode("+");
  }
}
const plusMarker = new PlusMarker();

/** The added lines (1-based numbers) to mark, replacing the previous set. */
export const setPlusLines = StateEffect.define<number[]>();

const plusField = StateField.define<RangeSet<GutterMarker>>({
  create: () => RangeSet.empty,
  update(value, tr) {
    for (const e of tr.effects) {
      if (e.is(setPlusLines)) {
        const doc = tr.state.doc;
        const at = e.value.filter((n) => n >= 1 && n <= doc.lines).map((n) => plusMarker.range(doc.line(n).from));
        return RangeSet.of(at, true);
      }
    }
    return tr.docChanged ? value.map(tr.changes) : value;
  },
});

const plusGutter = gutter({
  class: "cm-plus-gutter",
  markers: (view) => view.state.field(plusField),
  // A line being typed right now is an added line too.
  lineMarker(view, line, others) {
    if (others.length) return null;
    const live = view.state.field(liveField).marks;
    if (!live) return null;
    for (const a of live.add) if (a.to > a.from && a.from >= line.from && a.from <= line.to) return plusMarker;
    return null;
  },
  lineMarkerChange: (u) => u.docChanged || u.transactions.some((tr) => tr.effects.some((e) => e.is(setLiveMarks))),
  // Always as wide as a "+", so text does not shift when the first one appears.
  initialSpacer: () => plusMarker,
});

/** The lines marked "+" (1-based numbers), for the harness. */
export function plusLinesOf(state: EditorState): number[] {
  const out: number[] = [];
  const set = state.field(plusField, false);
  if (!set) return out;
  for (const it = set.iter(); it.value; it.next()) out.push(state.doc.lineAt(it.from).number);
  return out;
}

const theme = EditorView.theme({
  "&": { height: "100%", color: "var(--text)", backgroundColor: "var(--bg)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: 'ui-monospace, "JetBrains Mono", "D2Coding", monospace',
    fontSize: "0.92em",
    lineHeight: "1.5",
  },
  ".cm-content": { caretColor: "transparent", padding: "4px 0 40vh" },
  ".cm-gutters": {
    backgroundColor: "var(--surface)",
    color: "var(--muted)",
    border: "none",
    borderRight: "1px solid var(--border)",
  },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 8px 0 4px" },
  ".cm-plus-gutter .cm-gutterElement": {
    padding: "0 0 0 6px",
    color: "var(--green)",
    fontSize: "0.82em",
    fontWeight: "700",
    lineHeight: "1.83",
  },
  ".cm-live-zone": { boxShadow: "inset 3px 0 0 var(--accent)" },
  ".cm-live-del-line": { backgroundColor: "color-mix(in srgb, var(--red) 15%, transparent)" },
  ".cm-live-del": {
    textDecoration: "line-through",
    textDecorationColor: "var(--red)",
    color: "color-mix(in srgb, var(--text) 55%, var(--red))",
  },
  ".cm-live-add-line": { backgroundColor: "color-mix(in srgb, var(--green) 15%, transparent)" },
  ".cm-live-add": { backgroundColor: "color-mix(in srgb, var(--green) 22%, transparent)" },
  ".cm-live-settle": { animation: "amux-live-settle 1.6s ease-out forwards" },
  "@keyframes amux-live-settle": { to: { backgroundColor: "transparent" } },
  ".cm-live-caret": {
    color: "var(--accent)",
    fontWeight: "bold",
    animation: "amux-live-blink 0.9s steps(2) infinite",
  },
  "@keyframes amux-live-blink": { "50%": { opacity: "0" } },
});

const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.controlKeyword, t.moduleKeyword, t.operatorKeyword, t.definitionKeyword, t.modifier, t.self], color: "var(--accent)" },
  { tag: [t.string, t.special(t.string), t.regexp, t.character, t.docString], color: "var(--green)" },
  { tag: [t.number, t.bool, t.null, t.atom, t.unit], color: "var(--yellow)" },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: "var(--muted)", fontStyle: "italic" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.function(t.definition(t.variableName))], color: "var(--info)" },
  { tag: [t.typeName, t.className, t.namespace, t.definition(t.typeName), t.standard(t.typeName)], color: "var(--yellow)" },
  { tag: [t.propertyName, t.attributeName, t.labelName], color: "var(--text-2)" },
  { tag: [t.meta, t.annotation, t.processingInstruction, t.tagName], color: "var(--red)" },
  { tag: t.heading, color: "var(--accent)", fontWeight: "bold" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strong, fontWeight: "bold" },
  { tag: [t.link, t.url], color: "var(--info)", textDecoration: "underline" },
  { tag: t.invalid, color: "var(--red)" },
]);

const language = new Compartment();

const legacy = (load: () => Promise<StreamParser<unknown>>) => () =>
  load().then((parser) => StreamLanguage.define(parser) as Extension);

const BY_EXT: Record<string, () => Promise<Extension>> = {
  js: () => import("@codemirror/lang-javascript").then((m) => m.javascript()),
  mjs: () => import("@codemirror/lang-javascript").then((m) => m.javascript()),
  cjs: () => import("@codemirror/lang-javascript").then((m) => m.javascript()),
  jsx: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ jsx: true })),
  ts: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ typescript: true })),
  mts: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ typescript: true })),
  cts: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ typescript: true })),
  tsx: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ typescript: true, jsx: true })),
  py: () => import("@codemirror/lang-python").then((m) => m.python()),
  rs: () => import("@codemirror/lang-rust").then((m) => m.rust()),
  java: () => import("@codemirror/lang-java").then((m) => m.java()),
  json: () => import("@codemirror/lang-json").then((m) => m.json()),
  md: () => import("@codemirror/lang-markdown").then((m) => m.markdown()),
  markdown: () => import("@codemirror/lang-markdown").then((m) => m.markdown()),
  css: () => import("@codemirror/lang-css").then((m) => m.css()),
  scss: () => import("@codemirror/lang-css").then((m) => m.css()),
  less: () => import("@codemirror/lang-css").then((m) => m.css()),
  html: () => import("@codemirror/lang-html").then((m) => m.html()),
  htm: () => import("@codemirror/lang-html").then((m) => m.html()),
  vue: () => import("@codemirror/lang-html").then((m) => m.html()),
  svelte: () => import("@codemirror/lang-html").then((m) => m.html()),
  sql: () => import("@codemirror/lang-sql").then((m) => m.sql()),
  yaml: () => import("@codemirror/lang-yaml").then((m) => m.yaml()),
  yml: () => import("@codemirror/lang-yaml").then((m) => m.yaml()),
  kt: legacy(() => import("@codemirror/legacy-modes/mode/clike").then((m) => m.kotlin as StreamParser<unknown>)),
  kts: legacy(() => import("@codemirror/legacy-modes/mode/clike").then((m) => m.kotlin as StreamParser<unknown>)),
  go: legacy(() => import("@codemirror/legacy-modes/mode/go").then((m) => m.go as StreamParser<unknown>)),
  sh: legacy(() => import("@codemirror/legacy-modes/mode/shell").then((m) => m.shell as StreamParser<unknown>)),
  bash: legacy(() => import("@codemirror/legacy-modes/mode/shell").then((m) => m.shell as StreamParser<unknown>)),
  zsh: legacy(() => import("@codemirror/legacy-modes/mode/shell").then((m) => m.shell as StreamParser<unknown>)),
  toml: legacy(() => import("@codemirror/legacy-modes/mode/toml").then((m) => m.toml as StreamParser<unknown>)),
  xml: legacy(() => import("@codemirror/legacy-modes/mode/xml").then((m) => m.xml as StreamParser<unknown>)),
  svg: legacy(() => import("@codemirror/legacy-modes/mode/xml").then((m) => m.xml as StreamParser<unknown>)),
  properties: legacy(() => import("@codemirror/legacy-modes/mode/properties").then((m) => m.properties as StreamParser<unknown>)),
  ini: legacy(() => import("@codemirror/legacy-modes/mode/properties").then((m) => m.properties as StreamParser<unknown>)),
};

/** The syntax coloring for a file, by its name; null when unknown. */
export async function languageFor(path: string): Promise<Extension | null> {
  const name = path.slice(path.lastIndexOf("/") + 1);
  if (/^Dockerfile/i.test(name)) {
    return import("@codemirror/legacy-modes/mode/dockerfile").then((m) =>
      StreamLanguage.define(m.dockerFile as StreamParser<unknown>),
    );
  }
  const dot = name.lastIndexOf(".");
  const load = dot > 0 ? BY_EXT[name.slice(dot + 1).toLowerCase()] : undefined;
  if (!load) return null;
  try {
    return await load();
  } catch {
    return null;
  }
}

export function makeState(doc: string, lang: Extension | null = null): EditorState {
  return EditorState.create({
    doc,
    extensions: [
      plusField,
      plusGutter,
      lineNumbers(),
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      EditorState.tabSize.of(4),
      EditorView.lineWrapping,
      theme,
      syntaxHighlighting(highlight),
      liveField,
      language.of(lang ?? []),
    ],
  });
}

export function setLanguage(lang: Extension | null) {
  return language.reconfigure(lang ?? []);
}

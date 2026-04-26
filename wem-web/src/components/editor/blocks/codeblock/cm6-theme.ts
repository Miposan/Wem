/**
 * Wem CodeMirror 6 主题
 *
 * 核心布局原则：
 * - .cm-editor: height:auto，随内容自适应（无固定高度垂直滚动）
 * - .cm-scroller: 不覆盖 overflow，保留 CM6 原生的 sticky gutter 行为
 * - 水平溢出由 .cm-scroller 的原生 overflow:auto 处理
 * - gutter 通过 z-index 始终覆盖在 content 上层
 */
import { EditorView } from '@codemirror/view'

export const wemCmTheme = EditorView.theme({
  '&': {
    height: 'auto',
    fontSize: 'inherit',
  },

  // 不覆盖 .cm-scroller 的 overflow — CM6 默认 overflow:auto 处理横向滚动
  // .cm-editor height:auto → 垂直方向不会出现滚动条（内容撑高容器）
  '.cm-scroller': {
    fontFamily: 'inherit',
    lineHeight: '1.6',
  },

  '.cm-content': {
    padding: '0',
    caretColor: 'currentColor',
    fontFamily: "'JetBrains Mono', 'Fira Code', 'Consolas', 'Courier New', monospace",
    fontSize: '0.9em',
    tabSize: '2',
  },

  '.cm-line': {
    padding: '0',
  },

  '.cm-cursor': {
    borderLeftColor: 'currentColor',
    borderLeftWidth: '2px',
  },

  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
    backgroundColor: 'var(--color-fill-3, rgba(0, 0, 0, 0.08)) !important',
  },

  // Gutter — sticky 定位由 CM6 原生处理，只需确保 z-index 在 content 之上
  '.cm-gutters': {
    backgroundColor: 'color-mix(in oklab, var(--muted, #f5f5f5) 50%, var(--background, #fff))',
    borderRight: 'none',
    color: 'var(--color-text-3, #aaa)',
    fontSize: '0.8em',
    minWidth: '2em',
    paddingRight: '8px',
    position: 'sticky',
    left: '0',
    zIndex: '2',
  },

  '.cm-gutter': {
    minWidth: '2em',
  },

  '.cm-lineNumbers .cm-gutterElement': {
    padding: '0 4px 0 8px',
    textAlign: 'right',
  },

  '&.cm-focused': {
    outline: 'none',
  },
})

export const wemCmHighlight = EditorView.baseTheme({
  '&dark .cm-editor': {},
  '&light .cm-editor': {},
  '.tok-keyword': { color: 'var(--cm-keyword, #d73a49)' },
  '.tok-string': { color: 'var(--cm-string, #032f62)' },
  '.tok-number': { color: 'var(--cm-number, #005cc5)' },
  '.tok-comment': { color: 'var(--cm-comment, #6a737d)', fontStyle: 'italic' },
  '.tok-variableName': { color: 'var(--cm-variable, #e36209)' },
  '.tok-typeName': { color: 'var(--cm-type, #6f42c1)' },
  '.tok-propertyName': { color: 'var(--cm-property, #005cc5)' },
  '.tok-operator': { color: 'var(--cm-operator, #d73a49)' },
  '.tok-meta': { color: 'var(--cm-meta, #6a737d)' },
})

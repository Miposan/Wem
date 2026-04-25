/**
 * Wem CodeMirror 6 主题
 *
 * 设计原则：
 * - 代码块作为内嵌编辑区域，视觉上不与主文档冲突
 * - 背景色使用编辑器级别的 CSS 变量，自动适配深/浅色模式
 * - 使用轻量 gutter 显示行号，保持代码块可读性
 * - 字体与全局代码字体一致
 */
import { EditorView } from '@codemirror/view'

export const wemCmTheme = EditorView.theme({
  // ── 根容器：去掉 CM6 默认边框，让外层 .wem-codeblock 控制 ──
  '&': {
    height: 'auto',
    fontSize: 'inherit',
  },

  // ── 滚动容器：禁用 CM6 内部滚动，由块高度自适应 ──
  '.cm-scroller': {
    overflow: 'visible',
    fontFamily: 'inherit',
    lineHeight: '1.6',
  },

  // ── 内容区 ──
  '.cm-content': {
    padding: '0',
    caretColor: 'currentColor',
    fontFamily: "'JetBrains Mono', 'Fira Code', 'Consolas', 'Courier New', monospace",
    fontSize: '0.9em',
    tabSize: '2',
  },

  // ── 行 ──
  '.cm-line': {
    padding: '0',
  },

  // ── 光标 ──
  '.cm-cursor': {
    borderLeftColor: 'currentColor',
    borderLeftWidth: '2px',
  },

  // ── 选区 ──
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
    backgroundColor: 'var(--color-fill-3, rgba(0, 0, 0, 0.08)) !important',
  },

  // ── Gutter（行号） ──
  '.cm-gutters': {
    backgroundColor: 'transparent',
    borderRight: 'none',
    color: 'var(--color-text-3, #aaa)',
    fontSize: '0.8em',
    minWidth: '2em',
    paddingRight: '8px',
  },

  '.cm-gutter': {
    minWidth: '2em',
  },

  '.cm-lineNumbers .cm-gutterElement': {
    padding: '0 4px 0 8px',
    textAlign: 'right',
  },

  // ── 焦点轮廓：移除默认 outline ──
  '&.cm-focused': {
    outline: 'none',
  },
})

/**
 * Wem 代码块高亮
 *
 * 使用 CSS 变量控制语法高亮颜色，便于主题切换。
 * 没有定义变量的环境会回退到合理的默认值。
 */
export const wemCmHighlight = EditorView.baseTheme({
  '&dark .cm-editor': {
    // 深色模式暂由 oneDark 扩展处理，此处留空扩展点
  },
  '&light .cm-editor': {
    // 浅色模式暂由默认高亮处理，此处留空扩展点
  },
  // 通用 token 色彩（覆盖默认的蓝紫色）
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

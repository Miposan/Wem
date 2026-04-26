/**
 * CodeBlock — 基于 CodeMirror 6 的代码块编辑器
 *
 * 设计要点：
 * - 每个 CodeBlock 拥有独立的 CM6 EditorView 实例
 * - 高度随内容自适应（无固定高度滚动）
 * - 语言从 block_type.language 获取，切换时通过 Compartment 动态更新
 * - 内容变更通过 updateListener → onContentChange 同步到 Wem block 系统
 * - 外部内容变更（如 undo/redo）通过 dispatch 同步到 CM6
 *
 * 键盘集成：
 * - ArrowUp 在第一行第一列 → focus-previous
 * - ArrowDown 在最后一行末尾 → focus-next
 * - Backspace 在空块 → convert-block to paragraph
 * - Tab / Shift+Tab → 缩进/反缩进（由 CM6 indentWithTab 处理）
 * - Enter → 由 CM6 自行处理（自动换行，不触发 block split）
 */
import { useEffect, useRef, useCallback, useState } from 'react'
import { ChevronRight } from 'lucide-react'
import { EditorState, Compartment } from '@codemirror/state'
import type { Extension } from '@codemirror/state'
import { EditorView, keymap, placeholder as cmPlaceholder, lineNumbers } from '@codemirror/view'
import { indentWithTab, history } from '@codemirror/commands'
import { syntaxHighlighting, defaultHighlightStyle, bracketMatching } from '@codemirror/language'
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import { searchKeymap } from '@codemirror/search'

import type { BlockNode } from '@/types/api'
import type { BlockAction } from '../core/types'
import { makeParagraphType, makeCodeBlockType } from '@/types/api'
import { wemCmTheme, wemCmHighlight } from './codeblock/cm6-theme'
import { getLanguageExtension, getLanguageDisplayName, LANGUAGE_OPTIONS } from './codeblock/cm6-languages'

// ── Compartment：允许运行时替换的扩展槽 ──
const languageCompartment = new Compartment()
const readonlyCompartment = new Compartment()
const placeholderCompartment = new Compartment()

// ── 实例级同步标记：避免 updateListener 回调与外部同步形成循环 ──
const syncingViews = new WeakSet<EditorView>()

// ── Props ──

interface CodeBlockProps {
  block: BlockNode
  readonly: boolean
  placeholder?: string
  selectedBlockIds: ReadonlySet<string>
  onContentChange: (blockId: string, content: string) => void
  onAction: (action: BlockAction) => void
}

// ── 组件 ──

export function CodeBlock({
  block,
  readonly,
  placeholder,
  onContentChange,
  onAction,
}: CodeBlockProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const viewRef = useRef<EditorView | null>(null)

  // ── 折叠状态 ──
  const [collapsed, setCollapsed] = useState(false)

  // 稳定的回调引用：避免闭包陷阱导致 CM6 扩展重建
  const onContentChangeRef = useRef(onContentChange)
  onContentChangeRef.current = onContentChange

  const onActionRef = useRef(onAction)
  onActionRef.current = onAction

  const blockIdRef = useRef(block.id)
  blockIdRef.current = block.id

  const language = block.block_type.type === 'codeBlock' ? block.block_type.language : 'text'
  const content = block.content ?? ''

  // 统计行数（用于折叠提示）
  const lineCount = content ? content.split('\n').length : 0

  // ── 初始化 CM6 EditorView（block.id 变化时重建） ──
  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    // 清理旧实例（严格模式下 effect 会执行两次）
    if (viewRef.current) {
      syncingViews.delete(viewRef.current)
      viewRef.current.destroy()
      viewRef.current = null
    }

    const view = new EditorView({
      state: buildState({
        doc: content,
        language,
        readonly,
        placeholderText: placeholder || '输入代码…',
        onContentChangeRef,
        onActionRef,
        blockIdRef,
      }),
      parent: container,
    })

    viewRef.current = view

    return () => {
      syncingViews.delete(view)
      view.destroy()
      viewRef.current = null
    }
    // 仅 block.id 变化时重建实例；其他 props 通过 compartment 动态更新
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [block.id])

  // ── 外部内容同步：block.content 变更且非自身编辑触发 ──
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    if (syncingViews.has(view)) return

    const currentText = view.state.doc.toString()
    if (currentText === content) return

    syncingViews.add(view)
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: content },
    })
    // dispatch 是同步的，执行完后即可移除标记
    syncingViews.delete(view)
  }, [content])

  // ── 语言切换 ──
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({
      effects: languageCompartment.reconfigure(getLanguageExtension(language)),
    })
  }, [language])

  // ── readonly 切换 ──
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({
      effects: readonlyCompartment.reconfigure(EditorState.readOnly.of(readonly)),
    })
  }, [readonly])

  // ── placeholder 切换 ──
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({
      effects: placeholderCompartment.reconfigure(cmPlaceholder(placeholder || '输入代码…')),
    })
  }, [placeholder])

  // ── 语言切换：更新 block_type.language 并同步到后端 ──
  const handleLanguageChange = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      const newLang = e.target.value
      onActionRef.current({
        type: 'convert-block',
        blockId: block.id,
        content: block.content ?? '',
        blockType: makeCodeBlockType(newLang),
      })
    },
    [block.id, block.content],
  )

  const displayName = getLanguageDisplayName(language)

  return (
    <div className={`wem-codeblock${collapsed ? ' wem-codeblock-collapsed' : ''}`}>
      <div className="wem-codeblock-header">
        {/* 左侧：折叠按钮 + 语言标签（折叠时） */}
        {collapsed && (
          <span className="wem-codeblock-lang-label">{displayName}</span>
        )}
        {/* 右侧：折叠按钮 + 语言选择 */}
        <button
          className="wem-codeblock-collapse-btn"
          onClick={() => setCollapsed((c) => !c)}
          title={collapsed ? '展开代码块' : '折叠代码块'}
          tabIndex={-1}
        >
          <ChevronRight className={`wem-collapse-arrow${collapsed ? ' collapsed' : ''} h-3 w-3`} />
        </button>
        {!collapsed && !readonly ? (
          <select
            className="wem-codeblock-lang-select"
            value={language || 'text'}
            onChange={handleLanguageChange}
            tabIndex={-1}
          >
            {LANGUAGE_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        ) : !collapsed && readonly ? (
          <span className="wem-codeblock-lang-label">{displayName}</span>
        ) : null}
        {collapsed && lineCount > 0 && (
          <span className="wem-codeblock-collapse-hint">{lineCount} 行</span>
        )}
      </div>
      <div ref={containerRef} className="wem-codeblock-editor" />
    </div>
  )
}

// ── 构建 EditorState ──

interface BuildStateOptions {
  doc: string
  language: string
  readonly: boolean
  placeholderText: string
  onContentChangeRef: React.MutableRefObject<(blockId: string, content: string) => void>
  onActionRef: React.MutableRefObject<(action: BlockAction) => void>
  blockIdRef: React.MutableRefObject<string>
}

function buildState({
  doc,
  language,
  readonly,
  placeholderText,
  onContentChangeRef,
  onActionRef,
  blockIdRef,
}: BuildStateOptions): EditorState {
  const extensions: Extension[] = [
    // ── 编辑历史 ──
    history(),

    // ── 语言感知 ──
    bracketMatching(),
    closeBrackets(),

    // ── 行号 ──
    lineNumbers(),

    // ── 主题与高亮 ──
    wemCmTheme,
    wemCmHighlight,
    syntaxHighlighting(defaultHighlightStyle, { fallback: true }),

    // ── Compartments（运行时可替换） ──
    languageCompartment.of(getLanguageExtension(language)),
    readonlyCompartment.of(EditorState.readOnly.of(readonly)),
    placeholderCompartment.of(cmPlaceholder(placeholderText)),

    // ── 键映射 ──
    keymap.of([
      ...closeBracketsKeymap,
      ...searchKeymap,
      indentWithTab,
      // ── 块边界导航 ──
      {
        key: 'Mod-Enter',
        run: (view) => {
          onActionRef.current({
            type: 'exit-code-block',
            blockId: blockIdRef.current,
            content: view.state.doc.toString(),
          })
          return true
        },
      },
      {
        key: 'ArrowUp',
        run: (view) => {
          const pos = view.state.selection.main.head
          const line = view.state.doc.lineAt(pos)
          if (line.number === 1 && pos === line.from) {
            onActionRef.current({ type: 'focus-previous', blockId: blockIdRef.current })
            return true
          }
          return false
        },
      },
      {
        key: 'ArrowDown',
        run: (view) => {
          const pos = view.state.selection.main.head
          const line = view.state.doc.lineAt(pos)
          if (line.number === view.state.doc.lines && pos === line.to) {
            onActionRef.current({ type: 'focus-next', blockId: blockIdRef.current })
            return true
          }
          return false
        },
      },
      {
        key: 'Backspace',
        run: (view) => {
          if (view.state.doc.length === 0) {
            onActionRef.current({
              type: 'convert-block',
              blockId: blockIdRef.current,
              content: '',
              blockType: makeParagraphType(),
            })
            return true
          }
          return false
        },
      },
    ]),

    // ── 内容变更监听 ──
    EditorView.updateListener.of((update) => {
      if (!update.docChanged) return
      // 外部同步期间的变更不回调，避免循环
      if (syncingViews.has(update.view)) return
      onContentChangeRef.current(blockIdRef.current, update.state.doc.toString())
    }),
  ]

  return EditorState.create({ doc, extensions })
}

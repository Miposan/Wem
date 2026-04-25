import { useEffect, useState, useCallback, useRef } from 'react'
import { getDocument, updateBlock, type BlockNode } from '@/api/client'
import { WemEditor } from '@/components/editor'
import { EmojiPicker } from '@/components/editor/components/EmojiPicker'
import { TocPanel, extractTocItems, type TocItem } from '@/components/layout'
import { useTabStore } from '@/stores/tabStore'
import '@/components/editor/editor.css'

interface Props {
  documentId: string | null
}

export default function EditorPage({ documentId }: Props) {
  const [tree, setTree] = useState<BlockNode[]>([])
  const [title, setTitle] = useState('')
  const [icon, setIcon] = useState<string | undefined>(undefined)
  const [loading, setLoading] = useState(false)
  const [showToc, setShowToc] = useState(true)
  const { updateTab } = useTabStore()

  const handleTitleBlur = useCallback(
    (e: React.FocusEvent<HTMLHeadingElement>) => {
      const newTitle = e.currentTarget.textContent || ''
      setTitle(newTitle)
      if (documentId) {
        updateBlock(documentId, {
          properties: { title: newTitle },
          properties_mode: 'merge',
        }).catch((err) => console.error('标题保存失败:', err))
        updateTab(documentId, { title: newTitle })
      }
    },
    [documentId, updateTab],
  )

  /** Emoji 图标变更 */
  const handleIconChange = useCallback(
    (newIcon: string | undefined) => {
      setIcon(newIcon)
      if (documentId) {
        updateBlock(documentId, {
          properties: { ...(newIcon ? { icon: newIcon } : {}) },
          properties_mode: 'merge',
        }).catch((err) => console.error('图标保存失败:', err))
        updateTab(documentId, { icon: newIcon || '📄' })
      }
    },
    [documentId, updateTab],
  )

  /** TOC 标题点击 → 滚动到对应块 */
  const handleHeadingClick = useCallback((blockId: string) => {
    const el = document.querySelector(`[data-block-id="${blockId}"]`)
    if (el) {
      el.scrollIntoView({ behavior: 'smooth', block: 'center' })
    }
  }, [])

  useEffect(() => {
    if (!documentId) {
      setTree([])
      setTitle('')
      setIcon(undefined)
      return
    }
    setLoading(true)
    getDocument(documentId)
      .then((res) => {
        setTitle((res.document.properties?.title as string) || '')
        setIcon(res.document.properties?.icon as string | undefined)
        setTree(res.blocks)
      })
      .catch(console.error)
      .finally(() => setLoading(false))
  }, [documentId])

  if (!documentId) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <div className="text-center">
          <p className="text-6xl mb-4">📝</p>
          <p className="text-lg">选择或创建一个文档开始编辑</p>
        </div>
      </div>
    )
  }

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <p>加载中…</p>
      </div>
    )
  }

  // 提取 TOC
  const tocItems = extractTocItems(tree)

  return (
    <div className="flex-1 flex overflow-hidden">
      {/* 编辑器主区域 */}
      <main className="flex-1 overflow-y-auto">
        <div className="max-w-3xl mx-auto px-8 py-12">
          {/* Document Icon (Emoji) */}
          <div className="mb-4 relative">
            <EmojiPicker value={icon} onChange={handleIconChange}>
              {({ onClick, ref }) => (
                <div
                  ref={ref}
                  onClick={onClick}
                  className="w-16 h-16 flex items-center justify-center text-4xl rounded-lg hover:bg-accent/50 transition-colors cursor-pointer"
                  title="点击更换图标"
                >
                  {icon || '📄'}
                </div>
              )}
            </EmojiPicker>
          </div>

          {/* Document Title */}
          <h1
            className="text-3xl font-bold mb-8 outline-none"
            contentEditable
            suppressContentEditableWarning
            onBlur={handleTitleBlur}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                const firstBlock = document.querySelector('[data-block-id] [contenteditable]') as HTMLElement | null
                firstBlock?.focus()
              }
            }}
          >
            {title}
          </h1>

          {/* Wem Editor */}
          <WemEditor
            blocks={tree}
            documentId={documentId!}
            placeholder="输入内容，或输入 / 插入块…"
          />
        </div>
      </main>

      {/* 右侧 TOC 面板 */}
      {showToc && tocItems.length > 0 && (
        <aside className="w-56 h-full border-l border-border bg-muted/20 overflow-y-auto shrink-0">
          <div className="px-3 py-2 border-b border-border">
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-muted-foreground uppercase tracking-wider">目录</span>
              <button
                onClick={() => setShowToc(false)}
                className="text-xs text-muted-foreground hover:text-foreground cursor-pointer"
              >
                ×
              </button>
            </div>
          </div>
          <TocPanel items={tocItems} onHeadingClick={handleHeadingClick} />
        </aside>
      )}

      {/* TOC 切换按钮（当 TOC 隐藏时显示） */}
      {!showToc && tocItems.length > 0 && (
        <button
          onClick={() => setShowToc(true)}
          className="fixed right-4 top-16 w-8 h-8 flex items-center justify-center rounded bg-background border border-border shadow-sm text-muted-foreground hover:text-foreground transition-colors cursor-pointer z-10"
          title="显示目录"
        >
          ≡
        </button>
      )}
    </div>
  )
}

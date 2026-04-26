import { useEffect, useState, useCallback } from 'react'
import { getDocument, updateBlock } from '@/api/client'
import type { BlockNode } from '@/types/api'
import { WemEditor } from '@/components/editor'
import { EmojiPicker } from '@/components/editor/components/EmojiPicker'
import { extractTocItems, type TocItem } from '@/components/layout'
import { useTabStore } from '@/stores/tabStore'
import '@/components/editor/editor.css'

interface Props {
  documentId: string | null
  onTocItemsChange?: (items: TocItem[]) => void
}

// ─── 文档加载状态 ───
type DocData =
  | { status: 'idle' }
  | { status: 'loading'; docId: string }
  | { status: 'loaded'; docId: string; title: string; icon: string | undefined; tree: BlockNode[] }
  | { status: 'error'; docId: string; error: unknown }

export default function EditorPage({ documentId, onTocItemsChange }: Props) {
  const [doc, setDoc] = useState<DocData>({ status: 'idle' })
  const { updateTab } = useTabStore()

  const handleTitleBlur = useCallback(
    (e: React.FocusEvent<HTMLHeadingElement>) => {
      const newTitle = e.currentTarget.textContent || ''
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
      setDoc((prev) =>
        prev.status === 'loaded' ? { ...prev, icon: newIcon } : prev,
      )
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

  // 加载文档（只包含异步 setState，符合 React 19 规范）
  useEffect(() => {
    if (!documentId) return
    let cancelled = false
    // 异步加载数据，所有 setState 都在 .then 回调中
    getDocument(documentId)
      .then((res) => {
        if (cancelled) return
        const blocks = res.blocks
        const title = (res.document.properties?.title as string) || ''
        const icon = res.document.properties?.icon as string | undefined
        setDoc({ status: 'loaded', docId: documentId, title, icon, tree: blocks })
        onTocItemsChange?.(extractTocItems(blocks))
      })
      .catch((err) => {
        if (!cancelled) setDoc({ status: 'error', docId: documentId, error: err })
      })
    return () => { cancelled = true }
  }, [documentId, onTocItemsChange])

  // 无文档时显示空状态
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

  // 正在加载
  if (doc.status === 'loading' || (doc.status === 'idle' && documentId)) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <p>加载中…</p>
      </div>
    )
  }

  // 加载出错
  if (doc.status === 'error') {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <p>加载失败</p>
      </div>
    )
  }

  // 文档尚未加载完成（docId 不匹配）
  if (doc.status !== 'loaded' || doc.docId !== documentId) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <p>加载中…</p>
      </div>
    )
  }

  const { title, icon, tree } = doc

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
    </div>
  )
}

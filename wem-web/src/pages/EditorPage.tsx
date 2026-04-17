import { useEffect, useState, useCallback } from 'react'
import { getDocument, updateBlock, type BlockNode } from '@/api/client'
import { WemEditor } from '@/components/editor'
import '@/components/editor/editor.css'

interface Props {
  documentId: string | null
}

export default function EditorPage({ documentId }: Props) {
  const [tree, setTree] = useState<BlockNode[]>([])
  const [title, setTitle] = useState('')
  const [loading, setLoading] = useState(false)

  const handleTitleBlur = useCallback(
    (e: React.FocusEvent<HTMLHeadingElement>) => {
      const newTitle = e.currentTarget.textContent || ''
      setTitle(newTitle)
      // 持久化标题到后端：更新文档根块的 properties
      if (documentId) {
        updateBlock(documentId, {
          properties: { title: newTitle },
          properties_mode: 'merge',
        }).catch((err) => console.error('标题保存失败:', err))
      }
    },
    [documentId],
  )

  useEffect(() => {
    if (!documentId) {
      setTree([])
      setTitle('')
      return
    }
    setLoading(true)
    getDocument(documentId)
      .then((res) => {
        setTitle((res.document.properties?.title as string) || '')
        setTree(res.blocks)
      })
      .catch(console.error)
      .finally(() => setLoading(false))
  }, [documentId])

  if (!documentId) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <p>选择或创建一个文档开始编辑</p>
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

  return (
    <main className="flex-1 overflow-y-auto">
      <div className="max-w-3xl mx-auto px-8 py-12">
        {/* Document Title */}
        <h1
          className="text-3xl font-bold mb-8 outline-none"
          contentEditable
          suppressContentEditableWarning
          onBlur={handleTitleBlur}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              // 聚焦到编辑器第一个块
              const firstBlock = document.querySelector('[data-block-id] [contenteditable]') as HTMLElement | null
              firstBlock?.focus()
            }
          }}
        >
          {title}
        </h1>

        {/* Wem Editor — 自研编辑器内部处理保存 */}
        <WemEditor
          blocks={tree}
          documentId={documentId!}
          placeholder="输入内容，或输入 / 插入块…"
        />
      </div>
    </main>
  )
}

import { useEffect, useState } from 'react'
import { getDocument, type BlockNode } from '@/api/client'

interface Props {
  documentId: string | null
}

export default function EditorPage({ documentId }: Props) {
  const [tree, setTree] = useState<BlockNode[]>([])
  const [title, setTitle] = useState('')
  const [loading, setLoading] = useState(false)

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
        <h1 className="text-3xl font-bold mb-8 outline-none" contentEditable suppressContentEditableWarning>
          {title}
        </h1>

        {/* Block Tree (flat for now — will become rich editor) */}
        <div className="space-y-2">
          {tree.map((node) => (
            <BlockRenderer key={node.id} node={node} />
          ))}
        </div>

        {tree.length === 0 && (
          <p className="text-muted-foreground text-sm">
            文档为空，开始输入内容…
          </p>
        )}
      </div>
    </main>
  )
}

function BlockRenderer({ node }: { node: BlockNode }) {
  const typeTag = node.block_type.type

  const Tag =
    typeTag === 'heading'
      ? ((node.block_type as { type: 'heading'; level: number }).level === 1 ? 'h2' :
         (node.block_type as { type: 'heading'; level: number }).level === 2 ? 'h3' : 'h4')
      : typeTag === 'thematicBreak'
        ? 'hr'
        : 'p'

  if (typeTag === 'thematicBreak') {
    return <hr className="border-border my-4" />
  }

  return (
    <div className="group relative rounded hover:bg-accent/20 px-2 py-1 -mx-2 transition-colors">
      <Tag className="outline-none" contentEditable suppressContentEditableWarning>
        {node.content}
      </Tag>
      {node.children.length > 0 && (
        <div className="pl-4 border-l-2 border-border/50 space-y-1">
          {node.children.map((child) => (
            <BlockRenderer key={child.id} node={child} />
          ))}
        </div>
      )}
    </div>
  )
}

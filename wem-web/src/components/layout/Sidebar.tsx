import { useEffect, useState } from 'react'
import { listDocuments, createDocument, type Block } from '@/api/client'

interface SidebarProps {
  activeId: string | null
  onActiveChange: (id: string | null) => void
}

export function Sidebar({ activeId, onActiveChange }: SidebarProps) {
  const [documents, setDocuments] = useState<Block[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    listDocuments()
      .then((docs) => setDocuments(docs))
      .catch(console.error)
      .finally(() => setLoading(false))
  }, [])

  async function handleCreate() {
    const doc = await createDocument({ title: '无标题文档' })
    setDocuments((prev) => [...prev, doc])
    onActiveChange(doc.id)
  }

  return (
    <aside className="w-64 h-screen border-r border-border bg-muted/30 flex flex-col shrink-0">
      {/* Header */}
      <div className="flex items-center justify-between px-4 h-14 border-b border-border">
        <span className="font-semibold text-lg tracking-tight">Wem</span>
        <button
          onClick={handleCreate}
          className="text-sm px-2 py-1 rounded hover:bg-accent transition-colors cursor-pointer"
          title="新建文档"
        >
          +
        </button>
      </div>

      {/* Document List */}
      <nav className="flex-1 overflow-y-auto p-2 space-y-0.5">
        {loading && (
          <p className="text-sm text-muted-foreground px-2">加载中…</p>
        )}
        {!loading && documents.length === 0 && (
          <p className="text-sm text-muted-foreground px-2">暂无文档</p>
        )}
        {documents.map((doc) => (
          <button
            key={doc.id}
            onClick={() => onActiveChange(doc.id)}
            className={`w-full text-left px-3 py-2 rounded-md text-sm truncate transition-colors cursor-pointer ${
              activeId === doc.id
                ? 'bg-accent text-accent-foreground font-medium'
                : 'hover:bg-accent/50 text-foreground'
            }`}
          >
            {(doc.properties?.title as string) || '无标题'}
          </button>
        ))}
      </nav>
    </aside>
  )
}

import { useEffect, useState, useCallback } from 'react'
import { listDocuments, createDocument, deleteDocument, getDocumentChildren } from '@/api/client'
import { useTabStore } from '@/stores/tabStore'
import type { Block } from '@/types/api'

// ─── 树形文档节点 ───

interface DocTreeNode {
  doc: Block
  children: DocTreeNode[]
  loaded: boolean // 子节点是否已加载
}

// ─── Props ───

interface SidebarProps {
  activeId: string | null
  onActiveChange: (id: string | null) => void
  /** 嵌入模式：隐藏 header/footer，由外部 SlotContainer 管理折叠 */
  embedded?: boolean
}

// ─── 辅助函数 ───

/** 在树中查找节点 */
function findNode(tree: DocTreeNode[], id: string): DocTreeNode | null {
  for (const node of tree) {
    if (node.doc.id === id) return node
    const found = findNode(node.children, id)
    if (found) return found
  }
  return null
}

/** 在树中删除节点（返回新树） */
function removeNode(tree: DocTreeNode[], id: string): DocTreeNode[] {
  return tree
    .filter((n) => n.doc.id !== id)
    .map((n) => ({
      ...n,
      children: removeNode(n.children, id),
    }))
}

/** 在树中插入子节点 */
function addChild(tree: DocTreeNode[], parentId: string, child: DocTreeNode): DocTreeNode[] {
  return tree.map((n) => {
    if (n.doc.id === parentId) {
      return { ...n, children: [...n.children, child], loaded: true }
    }
    return { ...n, children: addChild(n.children, parentId, child) }
  })
}

/** 更新节点的 children 和 loaded 状态 */
function updateNodeChildren(
  tree: DocTreeNode[],
  parentId: string,
  children: DocTreeNode[],
): DocTreeNode[] {
  return tree.map((n) => {
    if (n.doc.id === parentId) {
      return { ...n, children, loaded: true }
    }
    return { ...n, children: updateNodeChildren(n.children, parentId, children) }
  })
}

/** 收集树中所有文档 ID */
function collectAllIds(tree: DocTreeNode[]): string[] {
  const ids: string[] = []
  for (const node of tree) {
    ids.push(node.doc.id)
    ids.push(...collectAllIds(node.children))
  }
  return ids
}

/** Block → DocTreeNode 转换 */
function toDocTreeNode(doc: Block, loaded = false): DocTreeNode {
  return { doc, children: [], loaded }
}

// ─── 文档项组件 ───

function DocItem({
  node,
  depth,
  activeId,
  expandedIds,
  onOpenDoc,
  onToggle,
  onCreateChild,
  onDelete,
}: {
  node: DocTreeNode
  depth: number
  activeId: string | null
  expandedIds: Set<string>
  onOpenDoc: (doc: Block) => void
  onToggle: (id: string) => void
  onCreateChild: (parentId: string) => void
  onDelete: (id: string) => void
}) {
  const isActive = activeId === node.doc.id
  const isExpanded = expandedIds.has(node.doc.id)
  const title = (node.doc.properties?.title as string) || '无标题'
  const icon = (node.doc.properties?.icon as string) || '📄'
  const [hovering, setHovering] = useState(false)

  return (
    <div>
      <div
        className={`group flex items-center rounded-md text-sm transition-colors ${
          isActive
            ? 'bg-accent text-accent-foreground font-medium'
            : 'hover:bg-accent/50 text-foreground'
        }`}
        style={{ paddingLeft: `${depth * 16 + 8}px`, paddingRight: '4px' }}
        onMouseEnter={() => setHovering(true)}
        onMouseLeave={() => setHovering(false)}
      >
        {/* 展开/折叠箭头 */}
        <button
          onClick={(e) => { e.stopPropagation(); onToggle(node.doc.id) }}
          className="shrink-0 w-5 h-7 flex items-center justify-center text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
        >
          <span className={`inline-block transition-transform text-xs ${isExpanded ? 'rotate-90' : ''}`}>
            ▶
          </span>
        </button>

        {/* 文档图标 */}
        <span className="shrink-0 text-sm mr-1.5">{icon}</span>

        {/* 文档标题 */}
        <button
          onClick={() => onOpenDoc(node.doc)}
          className="flex-1 text-left py-1.5 truncate cursor-pointer"
          title={title}
        >
          {title}
        </button>

        {/* 操作按钮（hover 显示） */}
        {hovering && (
          <div className="shrink-0 flex items-center gap-0.5">
            <button
              onClick={(e) => { e.stopPropagation(); onCreateChild(node.doc.id) }}
              className="w-5 h-5 flex items-center justify-center text-muted-foreground hover:text-foreground rounded hover:bg-accent/80 transition-colors cursor-pointer"
              title="添加子文档"
            >
              <span className="text-sm leading-none">+</span>
            </button>
            <button
              onClick={(e) => { e.stopPropagation(); onDelete(node.doc.id) }}
              className="w-5 h-5 flex items-center justify-center text-muted-foreground hover:text-red-500 rounded hover:bg-accent/80 transition-colors cursor-pointer"
              title="删除文档"
            >
              <span className="text-xs leading-none">×</span>
            </button>
          </div>
        )}
      </div>

      {/* 子文档 */}
      {isExpanded && node.loaded && node.children.length > 0 && (
        <div>
          {node.children.map((child) => (
            <DocItem
              key={child.doc.id}
              node={child}
              depth={depth + 1}
              activeId={activeId}
              expandedIds={expandedIds}
              onOpenDoc={onOpenDoc}
              onToggle={onToggle}
              onCreateChild={onCreateChild}
              onDelete={onDelete}
            />
          ))}
        </div>
      )}
    </div>
  )
}

// ─── 主侧边栏组件 ───

export function Sidebar({ activeId, onActiveChange, embedded }: SidebarProps) {
  const [tree, setTree] = useState<DocTreeNode[]>([])
  const [loading, setLoading] = useState(true)
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set())
  const [collapsed, setCollapsed] = useState(false)
  const { openTab } = useTabStore()

  // 加载根文档
  useEffect(() => {
    listDocuments()
      .then((docs) => setTree(docs.map((d) => toDocTreeNode(d))))
      .catch(console.error)
      .finally(() => setLoading(false))
  }, [])

  // 展开/折叠切换
  const handleToggle = useCallback(
    (docId: string) => {
      setExpandedIds((prev) => {
        const next = new Set(prev)
        if (next.has(docId)) {
          next.delete(docId)
        } else {
          next.add(docId)
          const node = findNode(tree, docId)
          if (node && !node.loaded) {
            getDocumentChildren(docId)
              .then((res) => {
                const children = res.children.map((d) => toDocTreeNode(d))
                setTree((prev) => updateNodeChildren(prev, docId, children))
              })
              .catch(console.error)
          }
        }
        return next
      })
    },
    [tree],
  )

  /** 打开文档 → 同时激活标签页 + 通知父组件 */
  const handleOpenDoc = useCallback(
    (doc: Block) => {
      const title = (doc.properties?.title as string) || '无标题'
      const icon = (doc.properties?.icon as string) || '📄'
      openTab({ id: doc.id, title, icon })
      onActiveChange(doc.id)
    },
    [openTab, onActiveChange],
  )

  const handleCreateRoot = useCallback(async () => {
    const doc = await createDocument({ title: '无标题文档' })
    setTree((prev) => [...prev, toDocTreeNode(doc)])
    handleOpenDoc(doc)
  }, [handleOpenDoc])

  const handleCreateChild = useCallback(
    async (parentId: string) => {
      const doc = await createDocument({ title: '无标题文档', parent_id: parentId })
      setTree((prev) => addChild(prev, parentId, toDocTreeNode(doc)))
      setExpandedIds((prev) => new Set(prev).add(parentId))
      handleOpenDoc(doc)
    },
    [handleOpenDoc],
  )

  const handleDelete = useCallback(
    async (docId: string) => {
      try {
        await deleteDocument(docId)
        setTree((prev) => removeNode(prev, docId))
        if (activeId === docId) {
          const remaining = collectAllIds(tree).filter((id) => id !== docId)
          onActiveChange(remaining.length > 0 ? remaining[0] : null)
        }
      } catch (err) {
        console.error('删除文档失败:', err)
      }
    },
    [activeId, onActiveChange, tree],
  )

  // ─── 嵌入模式：只渲染文档树 ───
  if (embedded) {
    return (
      <div className="flex flex-col h-full">
        {/* 文档树工具栏 */}
        <div className="flex items-center justify-between px-3 h-8 border-b border-border shrink-0">
          <span className="text-xs font-medium text-muted-foreground">文档</span>
          <button
            onClick={handleCreateRoot}
            className="w-5 h-5 flex items-center justify-center rounded hover:bg-accent text-muted-foreground hover:text-foreground transition-colors cursor-pointer text-sm"
            title="新建文档"
          >
            +
          </button>
        </div>
        {/* Document Tree */}
        <nav className="flex-1 overflow-y-auto p-2 space-y-0.5">
          {loading && <p className="text-sm text-muted-foreground px-2">加载中…</p>}
          {!loading && tree.length === 0 && (
            <div className="px-2 py-4 text-center">
              <p className="text-sm text-muted-foreground mb-2">暂无文档</p>
              <button
                onClick={handleCreateRoot}
                className="text-sm text-primary hover:underline cursor-pointer"
              >
                创建第一个文档
              </button>
            </div>
          )}
          {tree.map((node) => (
            <DocItem
              key={node.doc.id}
              node={node}
              depth={0}
              activeId={activeId}
              expandedIds={expandedIds}
              onOpenDoc={handleOpenDoc}
              onToggle={handleToggle}
              onCreateChild={handleCreateChild}
              onDelete={handleDelete}
            />
          ))}
        </nav>
      </div>
    )
  }

  // ─── 收起状态 ───
  if (collapsed) {
    return (
      <aside className="w-12 h-screen border-r border-border bg-muted/30 flex flex-col items-center pt-3 shrink-0">
        <button
          onClick={() => setCollapsed(false)}
          className="w-8 h-8 flex items-center justify-center rounded hover:bg-accent text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
          title="展开侧边栏"
        >
          ≡
        </button>
      </aside>
    )
  }

  return (
    <aside className="w-64 h-screen border-r border-border bg-muted/30 flex flex-col shrink-0">
      {/* Header */}
      <div className="flex items-center justify-between px-4 h-12 border-b border-border">
        <span className="font-semibold text-base tracking-tight">Wem</span>
        <div className="flex items-center gap-1">
          <button
            onClick={handleCreateRoot}
            className="w-7 h-7 flex items-center justify-center rounded hover:bg-accent text-muted-foreground hover:text-foreground transition-colors cursor-pointer text-sm"
            title="新建文档"
          >
            +
          </button>
          <button
            onClick={() => setCollapsed(true)}
            className="w-7 h-7 flex items-center justify-center rounded hover:bg-accent text-muted-foreground hover:text-foreground transition-colors cursor-pointer text-xs"
            title="收起侧边栏"
          >
            ◀
          </button>
        </div>
      </div>

      {/* Document Tree */}
      <nav className="flex-1 overflow-y-auto p-2 space-y-0.5">
        {loading && <p className="text-sm text-muted-foreground px-2">加载中…</p>}
        {!loading && tree.length === 0 && (
          <div className="px-2 py-4 text-center">
            <p className="text-sm text-muted-foreground mb-2">暂无文档</p>
            <button
              onClick={handleCreateRoot}
              className="text-sm text-primary hover:underline cursor-pointer"
            >
              创建第一个文档
            </button>
          </div>
        )}
        {tree.map((node) => (
          <DocItem
            key={node.doc.id}
            node={node}
            depth={0}
            activeId={activeId}
            expandedIds={expandedIds}
            onOpenDoc={handleOpenDoc}
            onToggle={handleToggle}
            onCreateChild={handleCreateChild}
            onDelete={handleDelete}
          />
        ))}
      </nav>

      {/* Footer */}
      <div className="px-4 py-2 border-t border-border text-xs text-muted-foreground">
        Wem Editor v0.1
      </div>
    </aside>
  )
}

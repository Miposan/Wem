import { useEffect, useState, useCallback } from 'react'
import { getBreadcrumb } from '@/api/client'
import type { BreadcrumbItem } from '@/api/client'

interface BreadcrumbProps {
  documentId: string
  onNavigate: (id: string, title: string, icon: string) => void
  /** 当前文档的最新标题，用于覆盖面包屑最后一项 */
  currentTitle?: string
}

export function Breadcrumb({ documentId, onNavigate, currentTitle }: BreadcrumbProps) {
  const [items, setItems] = useState<BreadcrumbItem[]>([])

  useEffect(() => {
    let cancelled = false
    getBreadcrumb(documentId)
      .then((result) => {
        if (cancelled) return
        setItems(result.items)
      })
      .catch(() => {
        if (!cancelled) setItems([])
      })
    return () => { cancelled = true }
  }, [documentId])

  // 本地标题变更时，直接覆盖最后一项的 title
  useEffect(() => {
    if (!currentTitle) return
    setItems((prev) => {
      if (prev.length === 0) return prev
      const last = prev[prev.length - 1]
      if (last.title === currentTitle) return prev
      const updated = [...prev]
      updated[updated.length - 1] = { ...last, title: currentTitle }
      return updated
    })
  }, [currentTitle])

  const handleClick = useCallback(
    (item: BreadcrumbItem) => {
      onNavigate(item.id, item.title, item.icon)
    },
    [onNavigate],
  )

  if (items.length === 0) return null

  return (
    <div className="wem-breadcrumb-bar">
      <div className="wem-breadcrumb-inner">
        {items.map((item, i) => {
          const isLast = i === items.length - 1
          return (
            <span key={item.id} className="inline-flex items-center">
              {i > 0 && <span className="wem-breadcrumb-arrow" />}
              <button
                className={`wem-breadcrumb-item ${isLast ? 'wem-breadcrumb-current' : ''}`}
                onClick={isLast ? undefined : () => handleClick(item)}
                title={item.title}
              >
                <span className="wem-breadcrumb-text">{item.title}</span>
              </button>
            </span>
          )
        })}
      </div>
    </div>
  )
}

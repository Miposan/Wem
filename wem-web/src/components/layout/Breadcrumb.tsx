import { useEffect, useState, useCallback } from 'react'
import { getBreadcrumb } from '@/api/client'
import type { BreadcrumbItem } from '@/api/client'

interface BreadcrumbProps {
  documentId: string
  onNavigate: (id: string, title: string, icon: string) => void
}

export function Breadcrumb({ documentId, onNavigate }: BreadcrumbProps) {
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

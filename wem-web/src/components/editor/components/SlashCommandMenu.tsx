import { useEffect, useRef } from 'react'
import { useSlashMenu, type SlashMenuItem } from '../core/SlashMenuContext'

interface SlashCommandMenuProps {
  onSelect: (item: SlashMenuItem, blockId: string, slashOffset: number, filterLen: number) => void
}

export function SlashCommandMenu({ onSelect }: SlashCommandMenuProps) {
  const ctx = useSlashMenu()
  const menuRef = useRef<HTMLDivElement>(null)

  const { state, filteredItems } = ctx

  useEffect(() => {
    if (!state.visible) return
    const el = menuRef.current?.querySelector('[data-selected="true"]')
    el?.scrollIntoView({ block: 'nearest' })
  }, [state.selectedIndex, state.visible])

  if (!state.visible || filteredItems.length === 0) return null

  return (
    <div
      ref={menuRef}
      className="wem-slash-menu"
      style={{ position: 'fixed', left: state.x, top: state.y + 6 }}
    >
      <div className="wem-slash-menu-section">基础块</div>
      {filteredItems.map((item, i) => (
        <button
          key={item.label}
          className={`wem-slash-menu-item ${i === state.selectedIndex ? 'wem-slash-menu-active' : ''}`}
          data-selected={i === state.selectedIndex}
          onMouseDown={(e) => {
            e.preventDefault()
            if (state.blockId) {
              onSelect(item, state.blockId, state.slashOffset, state.filter.length)
              ctx.close()
            }
          }}
          onMouseEnter={() => ctx.hoverIndex(i)}
        >
          <span className="wem-slash-menu-icon">{item.icon}</span>
          <div className="wem-slash-menu-info">
            <span className="wem-slash-menu-name">{item.label}</span>
            <span className="wem-slash-menu-desc">{item.description}</span>
          </div>
        </button>
      ))}
    </div>
  )
}

/**
 * TabBar — 多文档标签页栏
 *
 * 功能：
 * - 显示所有打开的文档标签
 * - 点击切换、中键关闭、右键菜单（关闭其他/关闭全部）
 * - 活跃标签高亮、溢出时可滚动
 */

import { useRef, useEffect, type MouseEvent as ReactMouseEvent } from 'react'
import { useTabStore, type TabInfo } from '@/stores/tabStore'

export function TabBar() {
  const { tabs, activeTabId, switchTab, closeTab, closeOtherTabs, closeAllTabs } = useTabStore()
  const scrollRef = useRef<HTMLDivElement>(null)

  // 活跃标签滚动到可见区域
  useEffect(() => {
    if (!scrollRef.current || !activeTabId) return
    const activeEl = scrollRef.current.querySelector(`[data-tab-id="${activeTabId}"]`)
    activeEl?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [activeTabId])

  // 中键关闭
  const handleAuxClick = (e: ReactMouseEvent, id: string) => {
    if (e.button === 1) {
      e.preventDefault()
      closeTab(id)
    }
  }

  // 右键菜单
  const handleContextMenu = (e: ReactMouseEvent, tab: TabInfo) => {
    e.preventDefault()
    // 简单实现：用浏览器原生 contextmenu 风格
    // TODO: 后续可换成自定义右键菜单组件
    const items = [
      { label: '关闭', action: () => closeTab(tab.id) },
      { label: '关闭其他', action: () => closeOtherTabs(tab.id) },
      { label: '关闭全部', action: () => closeAllTabs() },
    ]
    showTabContextMenu(e.clientX, e.clientY, items)
  }

  if (tabs.length === 0) return null

  return (
    <div className="flex items-center border-b border-border bg-background/80 backdrop-blur-sm">
      <div
        ref={scrollRef}
        className="flex-1 flex overflow-x-auto scrollbar-none"
      >
        {tabs.map((tab) => {
          const isActive = activeTabId === tab.id
          return (
            <div
              key={tab.id}
              data-tab-id={tab.id}
              className={`
                group flex items-center gap-1.5 px-3 h-9 min-w-[120px] max-w-[200px]
                border-r border-border cursor-pointer select-none
                transition-colors text-sm
                ${isActive
                  ? 'bg-background text-foreground'
                  : 'bg-muted/30 text-muted-foreground hover:bg-muted/60'
                }
              `}
              onClick={() => switchTab(tab.id)}
              onAuxClick={(e) => handleAuxClick(e, tab.id)}
              onContextMenu={(e) => handleContextMenu(e, tab)}
            >
              {/* 文档图标 */}
              <span className="text-base shrink-0">{tab.icon || '📄'}</span>

              {/* 标题 */}
              <span className="truncate flex-1">{tab.title || '无标题'}</span>

              {/* 关闭按钮 */}
              <button
                className={`
                  shrink-0 w-4 h-4 flex items-center justify-center rounded
                  text-muted-foreground hover:text-foreground hover:bg-accent
                  opacity-0 group-hover:opacity-100 transition-opacity
                  ${isActive ? 'opacity-100' : ''}
                `}
                onClick={(e) => {
                  e.stopPropagation()
                  closeTab(tab.id)
                }}
              >
                <span className="text-xs leading-none">×</span>
              </button>
            </div>
          )
        })}
      </div>
    </div>
  )
}

// ─── 简单的右键菜单（临时实现，后续替换为通用 ContextMenu 组件） ───

interface MenuItem {
  label: string
  action: () => void
}

let activeMenu: HTMLDivElement | null = null

function showTabContextMenu(x: number, y: number, items: MenuItem[]) {
  // 清除已有菜单
  if (activeMenu) activeMenu.remove()

  const menu = document.createElement('div')
  menu.className = 'fixed z-[9999] bg-popover border border-border rounded-md shadow-lg py-1 min-w-[140px]'
  menu.style.left = `${x}px`
  menu.style.top = `${y}px`

  items.forEach((item) => {
    const btn = document.createElement('button')
    btn.className = 'w-full text-left px-3 py-1.5 text-sm text-popover-foreground hover:bg-accent transition-colors'
    btn.textContent = item.label
    btn.onclick = () => {
      item.action()
      menu.remove()
      activeMenu = null
    }
    menu.appendChild(btn)
  })

  // 点击外部关闭
  const close = () => {
    menu.remove()
    activeMenu = null
    document.removeEventListener('click', close)
  }
  setTimeout(() => document.addEventListener('click', close), 0)

  document.body.appendChild(menu)
  activeMenu = menu
}

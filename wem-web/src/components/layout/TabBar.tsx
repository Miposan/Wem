/**
 * TabBar — 多文档标签页栏
 *
 * 功能：
 * - 显示所有打开的文档标签
 * - 点击切换、中键关闭、右键菜单（关闭其他/关闭全部）
 * - 活跃标签高亮、溢出时可滚动
 */

import { useRef, useEffect, type MouseEvent as ReactMouseEvent } from 'react'
import { X } from 'lucide-react'
import { useTabStore } from '@/stores/tabStore'
import { Button } from '@/components/ui/button'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'

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

  if (tabs.length === 0) return null

  return (
    <div className="flex items-center border-b border-border/50 bg-background">
      <div
        ref={scrollRef}
        className="flex-1 flex overflow-x-auto scrollbar-none"
      >
        {tabs.map((tab) => {
          const isActive = activeTabId === tab.id
          return (
            <ContextMenu key={tab.id}>
              <ContextMenuTrigger
                className={`
                  group flex items-center gap-1.5 px-3 h-8 min-w-[120px] max-w-[200px]
                  border-r border-border/30 cursor-pointer select-none
                  transition-colors text-sm
                  ${isActive
                    ? 'bg-background text-foreground'
                    : 'bg-transparent text-muted-foreground hover:bg-accent/40'
                  }
                `}
                data-tab-id={tab.id}
                onClick={() => switchTab(tab.id)}
                onAuxClick={(e) => handleAuxClick(e, tab.id)}
              >
                {/* 文档图标 */}
                <span className="text-base shrink-0">{tab.icon || '📄'}</span>

                {/* 标题 */}
                <span className="truncate flex-1">{tab.title || '无标题'}</span>

                {/* 关闭按钮 */}
                <Button
                  variant="ghost"
                  size="icon-xs"
                  className={`
                    shrink-0 rounded-sm
                    opacity-0 group-hover:opacity-100 transition-opacity
                    ${isActive ? 'opacity-100' : ''}
                  `}
                  onClick={(e: ReactMouseEvent) => {
                    e.stopPropagation()
                    closeTab(tab.id)
                  }}
                >
                  <X className="size-3" />
                </Button>
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem onClick={() => closeTab(tab.id)}>关闭</ContextMenuItem>
                <ContextMenuItem onClick={() => closeOtherTabs(tab.id)}>关闭其他</ContextMenuItem>
                <ContextMenuItem onClick={() => closeAllTabs()}>关闭全部</ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          )
        })}
      </div>
    </div>
  )
}

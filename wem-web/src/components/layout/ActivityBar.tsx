/**
 * ActivityBar — VS Code 风格活动栏
 *
 * 固定在侧边的窄栏（48px），永远可见。
 * - 显示属于该侧槽位的面板图标，点击切换面板的显示/隐藏
 * - 图标可拖拽到对侧 ActivityBar 来移动面板（ActivityBar 本身是 drop 目标）
 *
 * 图标和标题从 panelRegistry 获取，不再依赖 lucide-react 或 TooltipTrigger。
 */

import { useState, useCallback } from 'react'
import { Sun, Moon, Monitor } from 'lucide-react'
import { useLayoutStore, type PanelConfig, type SlotPosition } from '@/stores/layoutStore'
import { useTheme, type ThemeMode } from '@/theme'
import { getPanelDefinition } from './panelRegistry'

// ─── Props ───

export interface ActivityBarProps {
  side: 'left' | 'right'
}

// ─── 单个图标按钮（拖拽源） ───

function ActivityIcon({
  panel,
  active,
  side,
  onClick,
  dragging,
  onDragStart,
  onDragEnd,
}: {
  panel: PanelConfig
  active: boolean
  side: SlotPosition
  onClick: () => void
  dragging: boolean
  onDragStart: () => void
  onDragEnd: () => void
}) {
  const def = getPanelDefinition(panel.type)
  const Icon = def?.icon
  const title = def?.title ?? panel.id

  const handleDragStart = useCallback(
    (e: React.DragEvent) => {
      e.dataTransfer.effectAllowed = 'move'
      e.dataTransfer.setData('application/wem-panel', panel.id)
      e.dataTransfer.setData('application/wem-panel-source', side)
      onDragStart()
    },
    [onDragStart, panel.id, side],
  )

  const indicatorSide = side === 'left' ? 'left-0' : 'right-0'

  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      aria-pressed={active}
      draggable
      onClick={onClick}
      onDragStart={handleDragStart}
      onDragEnd={onDragEnd}
      className={`
        group relative flex items-center justify-center h-10 w-10 rounded-md p-0
        border-none bg-transparent cursor-pointer outline-none
        transition-[background-color,color,opacity] duration-100
        ${active
          ? 'text-foreground'
          : 'text-muted-foreground/60 hover:text-muted-foreground hover:bg-accent/50'
        }
        ${dragging ? 'opacity-40' : 'opacity-100'}
        focus-visible:ring-2 focus-visible:ring-ring
      `}
    >
      {/* active 指示条 */}
      <span
        className={`
          absolute ${indicatorSide} top-1/2 -translate-y-1/2 w-[3px] h-4 rounded-full
          bg-foreground/70 transition-opacity
          ${active ? 'opacity-100' : 'opacity-0'}
        `}
      />
      {Icon && <Icon className="h-5 w-5 shrink-0 pointer-events-none" />}
    </button>
  )
}

// ─── 活动栏组件（drop 目标） ───

export function ActivityBar({ side }: ActivityBarProps) {
  const { getAllSlotPanels, togglePanelToSlot, movePanel } = useLayoutStore()
  const { mode, setMode } = useTheme()
  const [dragOver, setDragOver] = useState(false)
  const [draggingPanelId, setDraggingPanelId] = useState<string | null>(null)

  // 该侧的所有面板（含隐藏的），保证拖拽过来时能看到图标
  const sidePanels = getAllSlotPanels(side)

  // ─── Drop 处理 ───

  const handleDragOver = useCallback(
    (e: React.DragEvent) => {
      if (!e.dataTransfer.types.includes('application/wem-panel')) return
      e.preventDefault()
      e.dataTransfer.dropEffect = 'move'
      setDragOver(true)
    },
    [],
  )

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    // 只在真正离开容器时取消高亮（避免进入子元素时闪烁）
    const rect = e.currentTarget.getBoundingClientRect()
    const { clientX, clientY } = e
    if (clientX < rect.left || clientX > rect.right || clientY < rect.top || clientY > rect.bottom) {
      setDragOver(false)
    }
  }, [])

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault()
      setDragOver(false)
      const panelId = e.dataTransfer.getData('application/wem-panel')
      const source = e.dataTransfer.getData('application/wem-panel-source')
      if (panelId && source !== side) {
        movePanel(panelId, side)
      }
    },
    [movePanel, side],
  )

  // ─── 主题切换（仅左侧活动栏底部显示） ───

  const cycleTheme = useCallback(() => {
    const next: Record<ThemeMode, ThemeMode> = { system: 'light', light: 'dark', dark: 'system' }
    setMode(next[mode])
  }, [mode, setMode])

  const ThemeIcon = mode === 'dark' ? Moon : mode === 'light' ? Sun : Monitor

  // ─── 渲染 ───

  const borderSide = side === 'left' ? 'border-r' : 'border-l'

  return (
    <div
      className={`
        relative h-full w-12 shrink-0 select-none flex flex-col items-center
        border-border/50 ${borderSide}
        bg-background
        transition-colors duration-150
        ${dragOver ? 'bg-accent' : ''}
      `}
      aria-label={side === 'left' ? '左侧活动栏' : '右侧活动栏'}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {/* 面板图标列表 */}
      <div className="flex flex-col items-center gap-0.5 pt-1">
        {sidePanels.map((panel) => (
          <ActivityIcon
            key={panel.id}
            panel={panel}
            active={panel.visible}
            side={side}
            onClick={() => togglePanelToSlot(panel.id, side)}
            dragging={draggingPanelId === panel.id}
            onDragStart={() => setDraggingPanelId(panel.id)}
            onDragEnd={() => {
              setDraggingPanelId(null)
              setDragOver(false)
            }}
          />
        ))}
      </div>

      {/* 空侧占位提示 */}
      {sidePanels.length === 0 && (
        <div className="mt-2 h-10 w-10 rounded-lg border border-dashed border-border/80 opacity-60" aria-hidden="true" />
      )}

      {/* 底部：主题切换（仅左侧） */}
      {side === 'left' && (
        <div className="mt-auto pb-2">
          <button
            type="button"
            title={`主题: ${mode === 'system' ? '跟随系统' : mode === 'dark' ? '暗色' : '亮色'}`}
            onClick={cycleTheme}
            className="flex items-center justify-center h-10 w-10 rounded-lg
              text-muted-foreground/50 hover:text-muted-foreground hover:bg-accent/40
              border-none bg-transparent cursor-pointer outline-none
              transition-[background-color,color] duration-150
              focus-visible:ring-2 focus-visible:ring-ring"
          >
            <ThemeIcon className="h-5 w-5 shrink-0" />
          </button>
        </div>
      )}

      {/* Drop 指示器 */}
      {dragOver && (
        <div className="absolute inset-0 border-2 border-dashed border-primary/50 rounded pointer-events-none z-30" />
      )}
    </div>
  )
}

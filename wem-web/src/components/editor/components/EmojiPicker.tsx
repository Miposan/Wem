/**
 * EmojiPicker — 文档图标选择器
 *
 * 参考 SiYuan 的文档图标选择器。
 * 点击文档图标弹出，选择 emoji 作为文档图标。
 * 支持：常用图标分组、搜索过滤
 */

import { useState, useRef, useEffect, useCallback } from 'react'

// ─── Emoji 数据 ───

const EMOJI_GROUPS: { label: string; emojis: string[] }[] = [
  {
    label: '常用',
    emojis: [
      '📄', '📝', '📋', '📌', '📎', '🔗', '💡', '🎯',
      '📂', '📁', '🗂️', '📰', '📖', '📚', '✏️', '🖊️',
    ],
  },
  {
    label: '自然',
    emojis: [
      '🌿', '🍀', '🌺', '🌸', '🌹', '🌻', '🌷', '🌱',
      '🌲', '🌳', '🌴', '🌵', '🌾', '🍁', '🍂', '🍃',
      '⭐', '🌟', '✨', '⚡', '🔥', '💧', '🌊', '🌈',
    ],
  },
  {
    label: '物品',
    emojis: [
      '💻', '🖥️', '⌨️', '📱', '💾', '💿', '📷', '🎥',
      '🎵', '🎶', '🎮', '🎲', '🧩', '🎨', '🖌️', '🔧',
      '🔨', '⚙️', '🔬', '🔭', '💡', '🔋', '🧲', '🔑',
    ],
  },
  {
    label: '符号',
    emojis: [
      '❤️', '💙', '💚', '💛', '💜', '🧡', '🖤', '🤍',
      '✅', '❌', '⚠️', '💯', '🔴', '🟠', '🟡', '🟢',
      '🔵', '🟣', '⬛', '⬜', '🔶', '🔷', '🔺', '🔻',
    ],
  },
  {
    label: '手势',
    emojis: [
      '👍', '👎', '👏', '🙌', '🤝', '✌️', '🤞', '🤟',
      '👌', '🤙', '💪', '🦾', '👀', '🧠', '🫀', '🦷',
    ],
  },
  {
    label: '交通',
    emojis: [
      '🚀', '✈️', '🚂', '🚃', '🚄', '🛸', '🛰️', '⛵',
      '🚗', '🚕', '🚌', '🚎', '🚑', '🚒', '🚓', '🏎️',
    ],
  },
]

// 所有 emoji 的扁平列表（用于搜索）
const ALL_EMOJIS = EMOJI_GROUPS.flatMap((g) => g.emojis)

// ─── Props ───

interface EmojiPickerProps {
  /** 当前选中的 emoji，undefined 表示无图标 */
  value?: string
  /** 选择回调 */
  onChange: (emoji: string | undefined) => void
  /** 触发器的 children（通常是图标显示区域） */
  children: (triggerProps: { onClick: () => void; ref: React.Ref<HTMLDivElement> }) => React.ReactNode
}

export function EmojiPicker({ value, onChange, children }: EmojiPickerProps) {
  const [open, setOpen] = useState(false)
  const [search, setSearch] = useState('')
  const [triggerHeight, setTriggerHeight] = useState(0)
  const triggerRef = useRef<HTMLDivElement>(null)
  const popoverRef = useRef<HTMLDivElement>(null)

  // 点击外部关闭
  useEffect(() => {
    if (!open) return
    const handleClickOutside = (e: MouseEvent) => {
      if (
        popoverRef.current && !popoverRef.current.contains(e.target as Node) &&
        triggerRef.current && !triggerRef.current.contains(e.target as Node)
      ) {
        setOpen(false)
        setSearch('')
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [open])

  const handleOpenToggle = useCallback(() => {
    setOpen((prev) => {
      if (!prev && triggerRef.current) {
        setTriggerHeight(triggerRef.current.offsetHeight)
      }
      return !prev
    })
    setSearch('')
  }, [])

  const handleSelect = useCallback((emoji: string) => {
    onChange(emoji === value ? undefined : emoji)
    setOpen(false)
    setSearch('')
  }, [value, onChange])

  const handleRemove = useCallback(() => {
    onChange(undefined)
    setOpen(false)
    setSearch('')
  }, [onChange])

  // 搜索过滤
  const filteredGroups = search
    ? [{ label: '搜索结果', emojis: ALL_EMOJIS }] // emoji 无法文本搜索，显示全部
    : EMOJI_GROUPS

  return (
    <>
      {children({
        ref: triggerRef,
        onClick: handleOpenToggle,
      })}

      {open && (
        <div
          ref={popoverRef}
          className="absolute z-50 w-[320px] bg-popover border border-border rounded-lg shadow-xl overflow-hidden"
          style={{
            top: triggerHeight + 4,
            left: 0,
          }}
        >
          {/* 搜索栏 */}
          <div className="p-2 border-b border-border">
            <input
              type="text"
              placeholder="搜索图标..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-full h-7 px-2 text-sm bg-background border border-border rounded outline-none focus:ring-1 focus:ring-ring"
              autoFocus
            />
          </div>

          {/* 已选 + 移除 */}
          {value && (
            <div className="flex items-center gap-2 px-3 py-2 border-b border-border bg-accent/30">
              <span className="text-2xl">{value}</span>
              <span className="text-sm text-muted-foreground flex-1">当前图标</span>
              <button
                onClick={handleRemove}
                className="text-xs text-muted-foreground hover:text-destructive transition-colors"
              >
                移除
              </button>
            </div>
          )}

          {/* Emoji 网格 */}
          <div className="max-h-[280px] overflow-y-auto p-2">
            {filteredGroups.map((group) => (
              <div key={group.label} className="mb-2">
                <div className="text-xs font-medium text-muted-foreground px-1 mb-1">
                  {group.label}
                </div>
                <div className="grid grid-cols-8 gap-0.5">
                  {group.emojis.map((emoji, i) => (
                    <button
                      key={`${emoji}-${i}`}
                      className={`
                        w-8 h-8 flex items-center justify-center rounded text-lg
                        hover:bg-accent transition-colors cursor-pointer
                        ${emoji === value ? 'bg-accent ring-1 ring-ring' : ''}
                      `}
                      onClick={() => handleSelect(emoji)}
                    >
                      {emoji}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </>
  )
}

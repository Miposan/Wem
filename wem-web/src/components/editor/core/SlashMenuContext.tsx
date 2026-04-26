import { createContext, useContext, useState, useCallback, type ReactNode } from 'react'
import { makeParagraphType, makeHeadingType, makeListType, makeCodeBlockType, makeMathBlockType } from '@/types/api'
import type { BlockType } from '@/types/api'

export interface SlashMenuItem {
  label: string
  description: string
  blockType: BlockType
  icon: string
}

export const SLASH_ITEMS: SlashMenuItem[] = [
  { label: '文本', description: '普通段落', blockType: makeParagraphType(), icon: 'T' },
  { label: '标题 1', description: '大标题', blockType: makeHeadingType(1), icon: 'H1' },
  { label: '标题 2', description: '中标题', blockType: makeHeadingType(2), icon: 'H2' },
  { label: '标题 3', description: '小标题', blockType: makeHeadingType(3), icon: 'H3' },
  { label: '无序列表', description: '项目符号列表', blockType: makeListType(false), icon: '•' },
  { label: '有序列表', description: '编号列表', blockType: makeListType(true), icon: '1.' },
  { label: '引用', description: '引用块', blockType: { type: 'blockquote' } as BlockType, icon: '❝' },
  { label: '代码块', description: '代码片段', blockType: makeCodeBlockType(''), icon: '</>' },
  { label: '公式', description: '数学公式', blockType: makeMathBlockType(), icon: 'fx' },
  { label: '分割线', description: '水平分割线', blockType: { type: 'thematicBreak' } as BlockType, icon: '—' },
]

export interface SlashMenuState {
  visible: boolean
  x: number
  y: number
  blockId: string | null
  slashOffset: number
  filter: string
  selectedIndex: number
}

interface SlashMenuContextValue {
  state: SlashMenuState
  filteredItems: SlashMenuItem[]
  trigger: (params: { blockId: string; x: number; y: number; slashOffset: number }) => void
  close: () => void
  setFilter: (filter: string) => void
  navigate: (direction: 'up' | 'down') => void
  hoverIndex: (index: number) => void
}

const SlashMenuContext = createContext<SlashMenuContextValue | null>(null)

export function useSlashMenu() {
  return useContext(SlashMenuContext)
}

function filterItems(filter: string): SlashMenuItem[] {
  if (!filter) return SLASH_ITEMS
  const lower = filter.toLowerCase()
  return SLASH_ITEMS.filter(
    (item) => item.label.toLowerCase().includes(lower) || item.description.toLowerCase().includes(lower),
  )
}

export function SlashMenuProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<SlashMenuState>({
    visible: false, x: 0, y: 0, blockId: null, slashOffset: 0, filter: '', selectedIndex: 0,
  })

  const filteredItems = filterItems(state.filter)

  const trigger = useCallback((params: { blockId: string; x: number; y: number; slashOffset: number }) => {
    setState({ visible: true, x: params.x, y: params.y, blockId: params.blockId, slashOffset: params.slashOffset, filter: '', selectedIndex: 0 })
  }, [])

  const close = useCallback(() => {
    setState((prev) => ({ ...prev, visible: false }))
  }, [])

  const setFilter = useCallback((filter: string) => {
    setState((prev) => ({ ...prev, filter, selectedIndex: 0 }))
  }, [])

  const navigate = useCallback((direction: 'up' | 'down') => {
    setState((prev) => {
      const max = filterItems(prev.filter).length - 1
      return { ...prev, selectedIndex: direction === 'up' ? Math.max(0, prev.selectedIndex - 1) : Math.min(max, prev.selectedIndex + 1) }
    })
  }, [])

  const hoverIndex = useCallback((index: number) => {
    setState((prev) => ({ ...prev, selectedIndex: index }))
  }, [])

  return (
    <SlashMenuContext.Provider value={{ state, filteredItems, trigger, close, setFilter, navigate, hoverIndex }}>
      {children}
    </SlashMenuContext.Provider>
  )
}

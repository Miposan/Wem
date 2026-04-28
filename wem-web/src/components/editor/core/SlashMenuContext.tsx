import { createContext, useContext, useState, useCallback, useMemo, useRef, type ReactNode } from 'react'
import { makeParagraphType, makeHeadingType, makeListType, makeCodeBlockType, makeMathBlockType, makeBlockquoteType, makeThematicBreakType, makeTableType } from '@/types/api'
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
  { label: '引用', description: '引用块', blockType: makeBlockquoteType(), icon: '❝' },
  { label: '代码块', description: '代码片段', blockType: makeCodeBlockType(''), icon: '</>' },
  { label: '公式', description: '数学公式', blockType: makeMathBlockType(), icon: 'fx' },
  { label: '表格', description: 'Markdown 表格', blockType: makeTableType(), icon: '⊞' },
  { label: '分割线', description: '水平分割线', blockType: makeThematicBreakType(), icon: '—' },
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

// ── Split context: dispatch (stable) + state (reactive) ──
// useTextBlock only subscribes to dispatch (never re-renders on state change).
// SlashCommandMenu subscribes to both (re-renders when state changes).

interface SlashMenuDispatch {
  trigger: (params: { blockId: string; x: number; y: number; slashOffset: number }) => void
  close: () => void
  setFilter: (filter: string) => void
  navigate: (direction: 'up' | 'down') => void
  hoverIndex: (index: number) => void
  /** Read state at call time (not reactive — for use in event handlers via refs) */
  getState: () => SlashMenuState
}

const SlashMenuDispatchContext = createContext<SlashMenuDispatch | null>(null)
const SlashMenuStateContext = createContext<SlashMenuState | null>(null)

export function filterItems(filter: string): SlashMenuItem[] {
  if (!filter) return SLASH_ITEMS
  const lower = filter.toLowerCase()
  return SLASH_ITEMS.filter(
    (item) => item.label.toLowerCase().includes(lower) || item.description.toLowerCase().includes(lower),
  )
}

/** Stable dispatch only — for useTextBlock (doesn't re-render on state changes) */
export function useSlashMenuDispatch(): SlashMenuDispatch {
  const ctx = useContext(SlashMenuDispatchContext)
  if (!ctx) throw new Error('useSlashMenuDispatch must be used within SlashMenuProvider')
  return ctx
}

/** Full context: state + dispatch + derived data — for SlashCommandMenu */
export function useSlashMenu(): {
  state: SlashMenuState
  filteredItems: SlashMenuItem[]
} & SlashMenuDispatch {
  const dispatch = useContext(SlashMenuDispatchContext)
  const state = useContext(SlashMenuStateContext)
  if (!dispatch || !state) throw new Error('useSlashMenu must be used within SlashMenuProvider')
  const filteredItems = useMemo(() => filterItems(state.filter), [state.filter])
  return { state, filteredItems, ...dispatch }
}

export function SlashMenuProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<SlashMenuState>({
    visible: false, x: 0, y: 0, blockId: null, slashOffset: 0, filter: '', selectedIndex: 0,
  })

  const stateRef = useRef(state)
  stateRef.current = state

  const filteredCountRef = useRef(SLASH_ITEMS.length)

  const dispatch = useMemo<SlashMenuDispatch>(() => ({
    trigger: (params) => {
      filteredCountRef.current = SLASH_ITEMS.length
      setState({ visible: true, x: params.x, y: params.y, blockId: params.blockId, slashOffset: params.slashOffset, filter: '', selectedIndex: 0 })
    },
    close: () => {
      setState((prev) => ({ ...prev, visible: false }))
    },
    setFilter: (filter) => {
      const items = filterItems(filter)
      filteredCountRef.current = items.length
      setState((prev) => ({ ...prev, filter, selectedIndex: 0 }))
    },
    navigate: (direction) => {
      setState((prev) => {
        const max = filteredCountRef.current - 1
        return { ...prev, selectedIndex: direction === 'up' ? Math.max(0, prev.selectedIndex - 1) : Math.min(max, prev.selectedIndex + 1) }
      })
    },
    hoverIndex: (index) => {
      setState((prev) => ({ ...prev, selectedIndex: index }))
    },
    getState: () => stateRef.current,
  }), [])

  return (
    <SlashMenuDispatchContext.Provider value={dispatch}>
      <SlashMenuStateContext.Provider value={state}>
        {children}
      </SlashMenuStateContext.Provider>
    </SlashMenuDispatchContext.Provider>
  )
}

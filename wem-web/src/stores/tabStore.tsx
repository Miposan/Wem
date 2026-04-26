/**
 * tabStore — 标签页状态管理
 *
 * 管理打开的文档标签页列表和当前活跃标签。
 * 使用 React state + context，无需外部状态库。
 */

import { createContext, useContext, useCallback, useState, type ReactNode } from 'react'

export interface TabInfo {
  id: string        // document id
  title: string
  icon?: string     // emoji icon
}

interface TabState {
  tabs: TabInfo[]
  activeTabId: string | null
}

interface TabActions {
  openTab: (tab: TabInfo) => void
  closeTab: (id: string) => void
  switchTab: (id: string) => void
  updateTab: (id: string, updates: Partial<TabInfo>) => void
  closeAllTabs: () => void
  closeOtherTabs: (id: string) => void
}

type TabStore = TabState & TabActions

const TabContext = createContext<TabStore | null>(null)

export function TabProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<TabState>({
    tabs: [],
    activeTabId: null,
  })

  const openTab = useCallback((tab: TabInfo) => {
    setState((prev) => {
      const existing = prev.tabs.find((t) => t.id === tab.id)
      if (existing) {
        // 已存在则切换
        return { ...prev, activeTabId: tab.id }
      }
      return {
        tabs: [...prev.tabs, tab],
        activeTabId: tab.id,
      }
    })
  }, [])

  const closeTab = useCallback((id: string) => {
    setState((prev) => {
      const idx = prev.tabs.findIndex((t) => t.id === id)
      if (idx === -1) return prev

      const newTabs = prev.tabs.filter((t) => t.id !== id)
      let newActiveId = prev.activeTabId

      if (prev.activeTabId === id) {
        // 关闭当前标签 → 切换到相邻标签
        if (newTabs.length === 0) {
          newActiveId = null
        } else if (idx < newTabs.length) {
          newActiveId = newTabs[idx].id
        } else {
          newActiveId = newTabs[newTabs.length - 1].id
        }
      }

      return { tabs: newTabs, activeTabId: newActiveId }
    })
  }, [])

  const switchTab = useCallback((id: string) => {
    setState((prev) => ({ ...prev, activeTabId: id }))
  }, [])

  const updateTab = useCallback((id: string, updates: Partial<TabInfo>) => {
    setState((prev) => ({
      ...prev,
      tabs: prev.tabs.map((t) => (t.id === id ? { ...t, ...updates } : t)),
    }))
  }, [])

  const closeAllTabs = useCallback(() => {
    setState({ tabs: [], activeTabId: null })
  }, [])

  const closeOtherTabs = useCallback((id: string) => {
    setState((prev) => {
      const tab = prev.tabs.find((t) => t.id === id)
      if (!tab) return prev
      return { tabs: [tab], activeTabId: id }
    })
  }, [])

  const store: TabStore = {
    ...state,
    openTab,
    closeTab,
    switchTab,
    updateTab,
    closeAllTabs,
    closeOtherTabs,
  }

  return <TabContext.Provider value={store}>{children}</TabContext.Provider>
}

export function useTabStore(): TabStore {
  const store = useContext(TabContext)
  if (!store) throw new Error('useTabStore must be used within TabProvider')
  return store
}

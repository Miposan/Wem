import { useState, useCallback } from 'react'
import { TabBar, SlotContainer, PanelContainer, ActivityBar } from '@/components/layout'
import { type TocItem } from '@/components/layout'
import { TabProvider, useTabStore } from '@/stores/tabStore'
import { LayoutProvider, useLayoutStore } from '@/stores/layoutStore'
import { TooltipProvider } from '@/components/ui/tooltip'
import { ThemeProvider } from '@/theme'
import { PanelContentRenderer } from '@/components/layout/panelRegistry'
import EditorPage from '@/pages/EditorPage'

function AppInner() {
  const [tocItems, setTocItems] = useState<TocItem[]>([])
  const { activeTabId, openTab, switchTab } = useTabStore()
  const { getSlotPanels } = useLayoutStore()

  const handleTocHeadingClick = useCallback((blockId: string) => {
    const el = document.querySelector(`[data-block-id="${blockId}"]`)
    if (el) el.scrollIntoView({ behavior: 'smooth', block: 'center' })
  }, [])

  const handleBreadcrumbNavigate = useCallback(
    (id: string, title: string, icon: string) => {
      openTab({ id, title, icon })
    },
    [openTab],
  )

  // 面板内容渲染所需的共享 props
  const contentProps = {
    tocItems,
    activeDocId: activeTabId,
    setActiveDocId: useCallback((id: string | null) => {
      if (id) switchTab(id)
    }, [switchTab]),
    onTocHeadingClick: handleTocHeadingClick,
  }

  // 获取各 slot 的面板
  const leftPanels = getSlotPanels('left')
  const rightPanels = getSlotPanels('right')
  const topPanels = getSlotPanels('top')

  const topVisible = topPanels.length > 0

  return (
    <div className="flex h-screen overflow-hidden bg-background text-foreground">
      {/* ─── Left Activity Bar ─── */}
      <ActivityBar side="left" />

      {/* ─── Left Slot ─── */}
      {leftPanels.length > 0 && (
        <SlotContainer slot="left">
          {leftPanels.map((panel) => (
            <PanelContainer key={panel.id} panel={panel}>
              <PanelContentRenderer panel={panel} {...contentProps} />
            </PanelContainer>
          ))}
        </SlotContainer>
      )}

      {/* ─── Center: Top Slot + Tabs + Editor ─── */}
      <div className="flex-1 flex flex-col min-w-0">
        {topVisible && (
          <SlotContainer slot="top">
            {topPanels.map((panel) => (
              <PanelContainer key={panel.id} panel={panel}>
                <PanelContentRenderer panel={panel} {...contentProps} />
              </PanelContainer>
            ))}
          </SlotContainer>
        )}

        {/* Tab Bar */}
        <TabBar />

        {/* Editor */}
        <EditorPage documentId={activeTabId} onTocItemsChange={setTocItems} onNavigate={handleBreadcrumbNavigate} />
      </div>

      {/* ─── Right Slot ─── */}
      {rightPanels.length > 0 && (
        <SlotContainer slot="right">
          {rightPanels.map((panel) => (
            <PanelContainer key={panel.id} panel={panel}>
              <PanelContentRenderer panel={panel} {...contentProps} />
            </PanelContainer>
          ))}
        </SlotContainer>
      )}

      {/* ─── Right Activity Bar ─── */}
      <ActivityBar side="right" />
    </div>
  )
}

function App() {
  return (
    <ThemeProvider>
      <LayoutProvider>
        <TabProvider>
          <TooltipProvider>
            <AppInner />
          </TooltipProvider>
        </TabProvider>
      </LayoutProvider>
    </ThemeProvider>
  )
}

export default App

import { useState } from 'react'
import { Sidebar, TabBar } from '@/components/layout'
import { TabProvider, useTabStore } from '@/stores/tabStore'
import EditorPage from '@/pages/EditorPage'

function AppInner() {
  const [activeDocId, setActiveDocId] = useState<string | null>(null)
  const { tabs } = useTabStore()

  // 活跃文档 ID：TabStore 优先，fallback 到 activeDocId
  const displayDocId = tabs.length > 0
    ? (tabs.find((t) => t.id === activeDocId) ? activeDocId : tabs[tabs.length - 1]?.id ?? null)
    : activeDocId

  return (
    <div className="flex h-screen bg-background text-foreground">
      <Sidebar activeId={displayDocId} onActiveChange={setActiveDocId} />

      {/* 主内容区域：标签栏 + 编辑器 */}
      <div className="flex-1 flex flex-col min-w-0">
        <TabBar />
        <EditorPage documentId={displayDocId} />
      </div>
    </div>
  )
}

function App() {
  return (
    <TabProvider>
      <AppInner />
    </TabProvider>
  )
}

export default App

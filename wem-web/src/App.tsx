import { useState } from 'react'
import { Sidebar } from '@/components/layout'
import EditorPage from '@/pages/EditorPage'

function App() {
  const [activeDocId, setActiveDocId] = useState<string | null>(null)

  return (
    <div className="flex h-screen bg-background text-foreground">
      <Sidebar activeId={activeDocId} onActiveChange={setActiveDocId} />
      <EditorPage documentId={activeDocId} />
    </div>
  )
}

export default App

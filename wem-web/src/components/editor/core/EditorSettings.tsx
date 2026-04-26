import { createContext, useContext } from 'react'

export interface EditorSettings {
  codeBlockWrap: boolean
}

const EditorSettingsContext = createContext<EditorSettings>({
  codeBlockWrap: false,
})

export function useEditorSettings(): EditorSettings {
  return useContext(EditorSettingsContext)
}

export function EditorSettingsProvider({
  value,
  children,
}: {
  value: EditorSettings
  children: React.ReactNode
}) {
  return (
    <EditorSettingsContext.Provider value={value}>
      {children}
    </EditorSettingsContext.Provider>
  )
}

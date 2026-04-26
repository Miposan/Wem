/**
 * 分隔线块 — 静态渲染，不可编辑
 *
 * 不可编辑但可选中：点击后可通过 Backspace/Delete 删除。
 */

interface ThematicBreakBlockProps {
  blockId: string
  onAction: (action: { type: 'delete'; blockId: string }) => void
}

export function ThematicBreakBlock({ blockId, onAction }: ThematicBreakBlockProps) {
  return (
    <hr
      className="wem-thematic-break"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Backspace' || e.key === 'Delete') {
          e.preventDefault()
          onAction({ type: 'delete', blockId })
        }
      }}
    />
  )
}

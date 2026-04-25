import type { TextBlockProps } from '../core/types'
import { useTextBlock } from '../core/useTextBlock'

/**
 * 列表项块
 *
 * 渲染为一个 <li>，复用 useTextBlock 的文本编辑逻辑。
 * 列表标记（bullet / number）由父 ListBlock 容器通过 CSS ::marker 或手动渲染。
 *
 * 键盘行为由 useTextBlock 处理：
 * - Enter → split（Commands 层会识别 ListItem 上下文，创建新 ListItem）
 * - Backspace 在空块 → delete（退出列表）
 */
export function ListItemBlock(props: TextBlockProps) {
  const { ref, handleInput, handleKeyDown, handleCompositionStart, handleCompositionEnd } = useTextBlock(props)

  return (
    <div
      ref={ref as React.RefObject<HTMLDivElement>}
      className="wem-list-item"
      contentEditable={!props.readonly}
      suppressContentEditableWarning
      data-placeholder={props.placeholder || '列表项…'}
      onInput={handleInput}
      onKeyDown={handleKeyDown}
      onCompositionStart={handleCompositionStart}
      onCompositionEnd={handleCompositionEnd}
    />
  )
}

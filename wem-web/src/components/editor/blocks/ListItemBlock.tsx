import type { TextBlockProps } from '../core/types'
import { useTextBlock, textBlockEditableProps } from '../core/useTextBlock'

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
  const tb = useTextBlock(props)

  return (
    <div
      ref={tb.ref as React.RefObject<HTMLDivElement>}
      className="wem-list-item"
      {...textBlockEditableProps(tb, props.readonly, props.placeholder || '列表项…')}
    />
  )
}

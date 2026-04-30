import type { TextBlockProps } from '../core/types'
import { useTextBlock, textBlockEditableProps } from '../core/useTextBlock'

/** 段落块 */
export function ParagraphBlock(props: TextBlockProps) {
  const tb = useTextBlock(props)

  return (
    <div
      ref={tb.ref as React.RefObject<HTMLDivElement>}
      className="wem-paragraph"
      {...textBlockEditableProps(tb, props.readonly, props.placeholder || '输入文字…')}
    />
  )
}

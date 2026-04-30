import type { TextBlockProps } from '../core/types'
import { useTextBlock, textBlockEditableProps } from '../core/useTextBlock'

/** 引用块 */
export function BlockquoteBlock(props: TextBlockProps) {
  const tb = useTextBlock(props)

  return (
    <blockquote
      ref={tb.ref as React.RefObject<HTMLQuoteElement>}
      className="wem-blockquote"
      {...textBlockEditableProps(tb, props.readonly, props.placeholder || '引用…')}
    />
  )
}

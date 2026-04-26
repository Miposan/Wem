import type { TextBlockProps } from '../core/types'
import { useTextBlock } from '../core/useTextBlock'

/** 引用块 */
export function BlockquoteBlock(props: TextBlockProps) {
  const { ref, handleInput, handleKeyDown, handlePaste, handleCompositionStart, handleCompositionEnd } = useTextBlock(props)

  return (
    <blockquote
      ref={ref as React.RefObject<HTMLQuoteElement>}
      className="wem-blockquote"
      contentEditable={!props.readonly}
      suppressContentEditableWarning
      data-placeholder={props.placeholder || '引用…'}
      onInput={handleInput}
      onKeyDown={handleKeyDown}
      onPaste={handlePaste}
      onCompositionStart={handleCompositionStart}
      onCompositionEnd={handleCompositionEnd}
    />
  )
}

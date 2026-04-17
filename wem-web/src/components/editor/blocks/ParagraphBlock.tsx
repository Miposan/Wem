import type { TextBlockProps } from '../core/types'
import { useTextBlock } from '../core/useTextBlock'

/** 段落块 */
export function ParagraphBlock(props: TextBlockProps) {
  const { ref, handleInput, handleKeyDown, handleCompositionStart, handleCompositionEnd } = useTextBlock(props)

  return (
    <div
      ref={ref as React.RefObject<HTMLDivElement>}
      className="wem-paragraph"
      contentEditable={!props.readonly}
      suppressContentEditableWarning
      data-placeholder={props.placeholder || '输入文字…'}
      onInput={handleInput}
      onKeyDown={handleKeyDown}
      onCompositionStart={handleCompositionStart}
      onCompositionEnd={handleCompositionEnd}
    />
  )
}

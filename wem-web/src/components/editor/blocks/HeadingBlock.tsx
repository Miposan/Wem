import type { TextBlockProps } from '../core/types'
import { useTextBlock } from '../core/useTextBlock'
import { getHeadingLevel } from '../core/BlockOperations'
import { useHeadingNumber } from '../core/HeadingNumbering'

const HEADING_TAGS = ['h1', 'h2', 'h3', 'h4', 'h5', 'h6'] as const

/** 标题块 (h1-h6) */
export function HeadingBlock({ block, readonly, placeholder, ...rest }: TextBlockProps) {
  const { ref, handleInput, handleKeyDown, handlePaste, handleCompositionStart, handleCompositionEnd } = useTextBlock({ block, readonly, placeholder, ...rest })
  const level = getHeadingLevel(block.block_type) ?? 2
  const Tag = HEADING_TAGS[level - 1] ?? 'h2'
  const number = useHeadingNumber(block.id)

  return (
    <Tag
      ref={ref as React.RefObject<HTMLHeadingElement>}
      className={`wem-heading wem-heading-${level}`}
      contentEditable={!readonly}
      suppressContentEditableWarning
      data-placeholder={placeholder || 'Heading'}
      data-heading-number={number ?? undefined}
      onInput={handleInput}
      onKeyDown={handleKeyDown}
      onPaste={handlePaste}
      onCompositionStart={handleCompositionStart}
      onCompositionEnd={handleCompositionEnd}
    />
  )
}

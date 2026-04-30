import type { TextBlockProps } from '../core/types'
import { useTextBlock, textBlockEditableProps } from '../core/useTextBlock'
import { getHeadingLevel } from '../core/BlockOperations'
import { useHeadingNumber } from '../core/HeadingNumbering'

const HEADING_TAGS = ['h1', 'h2', 'h3', 'h4', 'h5', 'h6'] as const

/** 标题块 (h1-h6) */
export function HeadingBlock({ block, readonly, placeholder, ...rest }: TextBlockProps) {
  const tb = useTextBlock({ block, readonly, placeholder, ...rest })
  const level = getHeadingLevel(block.block_type) ?? 2
  const Tag = HEADING_TAGS[level - 1] ?? 'h2'
  const number = useHeadingNumber(block.id)

  return (
    <Tag
      ref={tb.ref as React.RefObject<HTMLHeadingElement>}
      className={`wem-heading wem-heading-${level}`}
      data-heading-number={number ?? undefined}
      {...textBlockEditableProps(tb, readonly, placeholder || 'Heading')}
    />
  )
}

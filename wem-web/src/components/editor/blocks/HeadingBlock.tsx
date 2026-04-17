import type { TextBlockProps } from '../core/types'
import { useTextBlock } from '../core/useTextBlock'

/** 标题块 (h1-h6) */
export function HeadingBlock({ block, readonly, placeholder, ...rest }: TextBlockProps) {
  const { ref, handleInput, handleKeyDown } = useTextBlock({ block, readonly, placeholder, ...rest })
  const level = block.block_type.type === 'heading' ? (block.block_type as { level: number }).level : 2
  const Tag = `h${level}` as const

  return (
    <Tag
      ref={ref as React.RefObject<HTMLHeadingElement>}
      className={`wem-heading wem-heading-${level}`}
      contentEditable={!readonly}
      suppressContentEditableWarning
      data-placeholder={placeholder || 'Heading'}
      onInput={handleInput}
      onKeyDown={handleKeyDown}
    />
  )
}

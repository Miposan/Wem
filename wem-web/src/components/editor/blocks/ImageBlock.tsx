/**
 * ImageBlock — 图片块
 *
 * url 存储在 block_type.url 中，content 存储图片说明（caption）。
 */

import { useState, useCallback, useRef, useEffect } from 'react'
import type { BlockNode } from '@/types/api'
import type { BlockAction } from '../core/types'
import { ImageIcon } from 'lucide-react'

interface ImageBlockProps {
  block: BlockNode
  readonly: boolean
  onContentChange: (blockId: string, content: string) => void
  onAction: (action: BlockAction) => void
}

export function ImageBlock({ block, readonly, onContentChange }: ImageBlockProps) {
  const url = block.block_type.type === 'image' ? block.block_type.url : ''
  const caption = block.content ?? ''
  const [loaded, setLoaded] = useState(false)
  const [error, setError] = useState(false)
  const captionRef = useRef<HTMLParagraphElement>(null)

  useEffect(() => {
    if (captionRef.current && captionRef.current.textContent !== caption) {
      captionRef.current.textContent = caption
    }
  }, [block.id])

  const handleCaptionBlur = useCallback(() => {
    const el = captionRef.current
    if (!el) return
    const newCaption = el.textContent ?? ''
    if (newCaption !== (block.content ?? '')) {
      onContentChange(block.id, newCaption)
    }
  }, [block.id, block.content, onContentChange])

  if (!url) {
    return (
      <div className="wem-imageblock wem-imageblock-empty">
        <ImageIcon className="h-8 w-8 text-muted-foreground/40" />
        <span className="text-sm text-muted-foreground">图片 URL 为空</span>
      </div>
    )
  }

  return (
    <figure className="wem-imageblock">
      <div className="wem-imageblock-wrapper">
        {!loaded && !error && (
          <div className="wem-imageblock-loading">
            <div className="wem-imageblock-spinner" />
          </div>
        )}
        {error ? (
          <div className="wem-imageblock-error">
            <ImageIcon className="h-8 w-8 text-muted-foreground/40" />
            <span className="text-sm text-muted-foreground">图片加载失败</span>
          </div>
        ) : (
          <img
            src={url}
            alt={caption || '图片'}
            className="wem-imageblock-img"
            onLoad={() => setLoaded(true)}
            onError={() => setError(true)}
          />
        )}
      </div>
      {!readonly ? (
        <figcaption
          ref={captionRef}
          className="wem-imageblock-caption"
          contentEditable
          suppressContentEditableWarning
          onBlur={handleCaptionBlur}
        />
      ) : caption ? (
        <figcaption className="wem-imageblock-caption">{caption}</figcaption>
      ) : null}
    </figure>
  )
}

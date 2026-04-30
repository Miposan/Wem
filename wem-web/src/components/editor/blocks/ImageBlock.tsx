/**
 * ImageBlock — 图片块
 *
 * 数据模型：
 *   block.content  = ![caption](assets/xxx.png)
 *   block.properties.width = "50"  （百分比，可选）
 */

import { useState, useCallback, useRef, useEffect, useMemo } from 'react'
import type { BlockNode } from '@/types/api'
import type { BlockAction } from '../core/types'
import { uploadAsset, getAssetUrl, updateBlock } from '@/api/client'
import { ImageIcon, UploadIcon } from 'lucide-react'

// ─── Markdown 解析 ───

interface ParsedImage {
  caption: string
  url: string
}

function parseImageMarkdown(content: string): ParsedImage {
  const match = content.match(/^!\[([^\]]*)\]\(([^)]+)\)$/)
  if (match) return { caption: match[1], url: match[2] }
  if (content.startsWith('http')) return { caption: '', url: content }
  return { caption: '', url: '' }
}

function buildImageMarkdown(caption: string, url: string): string {
  return `![${caption}](${url})`
}

// ─── Props ───

interface ImageBlockProps {
  block: BlockNode
  readonly: boolean
  onContentChange: (blockId: string, content: string) => void
  onAction: (action: BlockAction) => void
}

// ─── 组件 ───

export function ImageBlock({ block, readonly, onContentChange }: ImageBlockProps) {
  const { caption, url } = useMemo(() => parseImageMarkdown(block.content ?? ''), [block.content])
  const widthPct = block.properties?.width ? Number(block.properties.width) : undefined
  const [loaded, setLoaded] = useState(false)
  const [error, setError] = useState(false)
  const [uploading, setUploading] = useState(false)
  const imgRef = useRef<HTMLImageElement>(null)
  const captionRef = useRef<HTMLParagraphElement>(null)
  const wrapperRef = useRef<HTMLDivElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const imageUrl = url ? getAssetUrl(url) : ''

  // URL 变化时重置状态
  useEffect(() => {
    setLoaded(false)
    setError(false)
  }, [imageUrl])

  // 检测浏览器缓存的图片（组件重建时 img.complete 已为 true）
  useEffect(() => {
    const img = imgRef.current
    if (img && img.complete && img.naturalWidth > 0) {
      setLoaded(true)
    }
  }, [imageUrl])

  // 同步 caption DOM
  useEffect(() => {
    if (captionRef.current && captionRef.current.textContent !== caption) {
      captionRef.current.textContent = caption
    }
  }, [block.id, caption])

  // ─── Caption ───

  const handleCaptionBlur = useCallback(() => {
    const el = captionRef.current
    if (!el) return
    const newCaption = el.textContent ?? ''
    if (newCaption !== caption) {
      onContentChange(block.id, buildImageMarkdown(newCaption, url))
    }
  }, [block.id, caption, url, onContentChange])

  // ─── 文件上传 ───

  const handleUpload = useCallback(async (file: File) => {
    if (readonly || uploading) return
    setUploading(true)
    try {
      const path = await uploadAsset(file)
      onContentChange(block.id, buildImageMarkdown(caption, path))
    } catch (err) {
      console.error('图片上传失败:', err)
    } finally {
      setUploading(false)
    }
  }, [block.id, caption, readonly, uploading, onContentChange])

  const handleFileSelect = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (file) handleUpload(file)
    e.target.value = ''
  }, [handleUpload])

  // ─── 拖放 ───

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    e.stopPropagation()
    const file = e.dataTransfer.files[0]
    if (file?.type.startsWith('image/')) handleUpload(file)
  }, [handleUpload])

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault()
  }, [])

  // ─── 宽度调整 ───

  const handleResizeStart = useCallback((e: React.MouseEvent, side: 'left' | 'right') => {
    e.preventDefault()
    const startX = e.clientX
    const wrapper = wrapperRef.current
    if (!wrapper) return
    const startWidth = wrapper.getBoundingClientRect().width
    const editorWidth = wrapper.parentElement?.getBoundingClientRect().width ?? startWidth

    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'

    const handleMove = (ev: MouseEvent) => {
      const dx = ev.clientX - startX
      const multiplier = side === 'left' ? -2 : 2
      const newPct = Math.max(10, Math.min(100, (startWidth + dx * multiplier) / editorWidth * 100))
      wrapper.style.width = `${newPct}%`
    }

    const handleUp = () => {
      document.removeEventListener('mousemove', handleMove)
      document.removeEventListener('mouseup', handleUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
      const finalWidth = wrapper.getBoundingClientRect().width
      const finalPct = Math.round(Math.max(10, Math.min(100, finalWidth / editorWidth * 100)))
      updateBlock(block.id, { properties: { width: String(finalPct) } }).catch(console.error)
    }

    document.addEventListener('mousemove', handleMove)
    document.addEventListener('mouseup', handleUp)
  }, [block.id])

  // ─── 空状态 ───

  if (!url) {
    return (
      <div
        className="wem-imageblock wem-imageblock-empty"
        onDrop={handleDrop}
        onDragOver={handleDragOver}
      >
        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          className="hidden"
          onChange={handleFileSelect}
        />
        {uploading ? (
          <>
            <div className="wem-imageblock-spinner" />
            <span className="text-sm text-muted-foreground">上传中…</span>
          </>
        ) : (
          <>
            <ImageIcon className="h-8 w-8 text-muted-foreground/40" />
            <span className="text-sm text-muted-foreground">拖拽图片或点击上传</span>
            <button
              className="mt-2 px-3 py-1.5 text-sm text-muted-foreground hover:text-foreground border border-border rounded hover:bg-accent transition-colors cursor-pointer"
              onClick={() => fileInputRef.current?.click()}
            >
              <UploadIcon className="h-3.5 w-3.5 inline mr-1" />
              选择文件
            </button>
          </>
        )}
      </div>
    )
  }

  // ─── 图片渲染 ───

  return (
    <figure
      className="wem-imageblock"
      onDrop={handleDrop}
      onDragOver={handleDragOver}
    >
      <div
        ref={wrapperRef}
        className="wem-imageblock-wrapper"
        style={widthPct ? { width: `${widthPct}%` } : undefined}
      >
        {/* 图片始终在 DOM 中，这样浏览器缓存能直接触发 onLoad */}
        {!error && (
          <img
            ref={imgRef}
            src={imageUrl}
            alt={caption || '图片'}
            className="wem-imageblock-img"
            onLoad={() => setLoaded(true)}
            onError={() => setError(true)}
          />
        )}

        {/* 加载中：绝对定位叠加层 */}
        {!loaded && !error && (
          <div className="wem-imageblock-loading">
            <div className="wem-imageblock-spinner" />
          </div>
        )}

        {/* 加载失败 */}
        {error && (
          <div className="wem-imageblock-error">
            <ImageIcon className="h-8 w-8 text-muted-foreground/40" />
            <span className="text-sm text-muted-foreground">图片加载失败</span>
          </div>
        )}

        {/* 宽度把手 */}
        {!readonly && loaded && !error && (
          <>
            <div
              className="wem-imageblock-resize-handle left"
              onMouseDown={(e) => handleResizeStart(e, 'left')}
            />
            <div
              className="wem-imageblock-resize-handle right"
              onMouseDown={(e) => handleResizeStart(e, 'right')}
            />
          </>
        )}
      </div>

      {/* Caption */}
      {!readonly ? (
        <p
          ref={captionRef}
          className={`wem-imageblock-caption ${caption ? 'visible' : ''}`}
          contentEditable
          suppressContentEditableWarning
          onBlur={handleCaptionBlur}
          onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); (e.target as HTMLElement).blur() } }}
          data-placeholder="添加图片说明…"
        />
      ) : caption ? (
        <p className="wem-imageblock-caption visible">{caption}</p>
      ) : null}
    </figure>
  )
}

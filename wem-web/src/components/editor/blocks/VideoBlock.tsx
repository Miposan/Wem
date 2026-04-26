/**
 * VideoBlock — 视频块
 *
 * url 存储在 block_type.url 中。
 * 支持原生 <video> 和 YouTube/Bilibili 等外链嵌入。
 */

import { useState } from 'react'
import type { BlockNode } from '@/types/api'
import { VideoIcon } from 'lucide-react'

interface VideoBlockProps {
  block: BlockNode
  readonly: boolean
}

/** 判断是否为嵌入型视频链接（YouTube / Bilibili / Vimeo） */
function getEmbedUrl(url: string): string | null {
  // YouTube
  const ytMatch = url.match(/(?:youtube\.com\/watch\?v=|youtu\.be\/)([\w-]+)/)
  if (ytMatch) return `https://www.youtube.com/embed/${ytMatch[1]}`

  // Bilibili
  const biliMatch = url.match(/bilibili\.com\/video\/(BV[\w]+)/)
  if (biliMatch) return `https://player.bilibili.com/player.html?bvid=${biliMatch[1]}`

  // Vimeo
  const vimeoMatch = url.match(/vimeo\.com\/(\d+)/)
  if (vimeoMatch) return `https://player.vimeo.com/video/${vimeoMatch[1]}`

  return null
}

export function VideoBlock({ block }: VideoBlockProps) {
  const url = block.block_type.type === 'video' ? block.block_type.url : ''
  const [error, setError] = useState(false)

  if (!url) {
    return (
      <div className="wem-videoblock wem-videoblock-empty">
        <VideoIcon className="h-8 w-8 text-muted-foreground/40" />
        <span className="text-sm text-muted-foreground">视频 URL 为空</span>
      </div>
    )
  }

  const embedUrl = getEmbedUrl(url)

  if (error) {
    return (
      <div className="wem-videoblock wem-videoblock-error">
        <VideoIcon className="h-8 w-8 text-muted-foreground/40" />
        <span className="text-sm text-muted-foreground">视频加载失败</span>
      </div>
    )
  }

  if (embedUrl) {
    return (
      <div className="wem-videoblock">
        <div className="wem-videoblock-wrapper">
          <iframe
            src={embedUrl}
            className="wem-videoblock-iframe"
            allowFullScreen
            sandbox="allow-scripts allow-same-origin allow-presentation"
          />
        </div>
      </div>
    )
  }

  return (
    <div className="wem-videoblock">
      <div className="wem-videoblock-wrapper">
        <video
          src={url}
          className="wem-videoblock-video"
          controls
          onError={() => setError(true)}
        />
      </div>
    </div>
  )
}

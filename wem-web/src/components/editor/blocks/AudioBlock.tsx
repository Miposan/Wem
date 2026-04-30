import { useEffect, useState } from 'react'
import type { BlockNode } from '@/types/api'
import { Music } from 'lucide-react'

interface AudioBlockProps {
  block: BlockNode
  readonly: boolean
}

export function AudioBlock({ block }: AudioBlockProps) {
  const url = block.block_type.type === 'audio' ? block.block_type.url : ''
  const [error, setError] = useState(false)

  useEffect(() => {
    setError(false)
  }, [url])

  if (!url) {
    return (
      <div className="wem-audioblock wem-audioblock-empty">
        <Music className="h-8 w-8 text-muted-foreground/40" />
        <span className="text-sm text-muted-foreground">音频 URL 为空</span>
      </div>
    )
  }

  if (error) {
    return (
      <div className="wem-audioblock wem-audioblock-error">
        <Music className="h-8 w-8 text-muted-foreground/40" />
        <span className="text-sm text-muted-foreground">音频加载失败</span>
      </div>
    )
  }

  return (
    <div className="wem-audioblock">
      <audio
        src={url}
        className="wem-audioblock-player"
        controls
        onError={() => setError(true)}
      />
    </div>
  )
}

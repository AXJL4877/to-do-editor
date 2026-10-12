import type { RefObject } from 'react'

type Props = {
  src: string | null
  videoRef: RefObject<HTMLVideoElement | null>
  width?: number
  height?: number
  onLoadedMetadata: () => void
  onTimeUpdate: () => void
  onPlay: () => void
  onPause: () => void
  onEnded: () => void
}

export function ProgramMonitor({
  src,
  videoRef,
  width,
  height,
  onLoadedMetadata,
  onTimeUpdate,
  onPlay,
  onPause,
  onEnded,
}: Props) {
  const ratio =
    typeof width === 'number' &&
    typeof height === 'number' &&
    width > 0 &&
    height > 0
      ? `${width} / ${height}`
      : undefined

  return (
    <main className="monitor">
      <div className="panel-label">节目监视器</div>
      <div className="monitor-stage">
        {src ? (
          <video
            ref={videoRef}
            className="monitor-video"
            src={src}
            playsInline
            preload="auto"
            style={ratio ? { aspectRatio: ratio } : undefined}
            onLoadedMetadata={onLoadedMetadata}
            onTimeUpdate={onTimeUpdate}
            onPlay={onPlay}
            onPause={onPause}
            onEnded={onEnded}
          />
        ) : (
          <div className="monitor-empty">
            <div className="monitor-empty-icon" aria-hidden />
            <span>导入视频后在此预览</span>
          </div>
        )}
      </div>
    </main>
  )
}

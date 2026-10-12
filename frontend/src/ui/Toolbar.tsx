import { useRef } from 'react'
import { AppButton } from './AppButton'

type Props = {
  projectName: string
  clipName: string | null
  currentTime: number
  duration: number
  playing: boolean
  canPlay: boolean
  onImport: (file: File | undefined) => void
  onTogglePlay: () => void
  onBack: () => void
  onSave: () => void
}

export function Toolbar({
  projectName,
  clipName,
  currentTime,
  duration,
  playing,
  canPlay,
  onImport,
  onTogglePlay,
  onBack,
  onSave,
}: Props) {
  const inputRef = useRef<HTMLInputElement>(null)

  return (
    <header className="top">
      <div className="top-left">
        <span className="brand">TO DO</span>
        <div className="btn-group">
          <AppButton variant="ghost" onClick={onBack}>工程</AppButton>
          <AppButton onClick={onSave}>保存</AppButton>
          <AppButton onClick={() => inputRef.current?.click()}>导入媒体</AppButton>
          <input
            ref={inputRef}
            type="file"
            accept="video/*"
            hidden
            onChange={(e) => onImport(e.target.files?.[0])}
          />
        </div>
      </div>
      <div className="top-center">
        <AppButton variant="primary" disabled={!canPlay} onClick={onTogglePlay}>
          {playing ? '暂停' : '播放'}
        </AppButton>
      </div>
      <div className="top-right">
        <div className="meta-block">
          <div className="meta-title">{projectName}</div>
          <div className="meta-sub">
            {clipName ? `${clipName} · ${fmt(currentTime)} / ${fmt(duration)}` : '未导入媒体'}
          </div>
        </div>
      </div>
    </header>
  )
}

function fmt(sec: number) {
  if (!Number.isFinite(sec) || sec < 0) return '0:00'
  const m = Math.floor(sec / 60)
  const s = Math.floor(sec % 60)
  return `${m}:${String(s).padStart(2, '0')}`
}

import { useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import type { Project } from '../../../core/project'
import { move } from '../../../core/timeline/actions/move'
import { razor } from '../../../core/timeline/actions/razor'
import { stretch } from '../../../core/timeline/actions/stretch'
import { trim, type TrimEdge } from '../../../core/timeline/actions/trim'
import type { Clip } from '../../../core/timeline/clip'

const PX_PER_SEC = 48
const LABEL_W = 120
const EDGE_PX = 8

type DragMode =
  | { kind: 'move'; clipId: string; originStart: number; originX: number }
  | {
      kind: 'trim' | 'stretch'
      clipId: string
      edge: TrimEdge
      mediaDuration?: number
    }

type Props = {
  project: Project
  currentTime: number
  onSeek: (time: number) => void
  onChange: (project: Project) => void
}

export function Timeline({ project, currentTime, onSeek, onChange }: Props) {
  const boardRef = useRef<HTMLDivElement>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const dragRef = useRef<DragMode | null>(null)
  const projectRef = useRef(project)
  projectRef.current = project

  const axis = useMemo(() => {
    let max = typeof project.duration === 'number' ? project.duration : 10
    for (const track of project.tracks) {
      for (const clip of track.clips) {
        max = Math.max(max, clip.start + clip.duration)
      }
    }
    for (const m of project.media) max = Math.max(max, m.duration)
    return Math.max(max, 10)
  }, [project])

  const laneWidth = Math.max(axis * PX_PER_SEC, 640)
  const playheadX = LABEL_W + currentTime * PX_PER_SEC
  const ticks = Array.from({ length: Math.floor(axis) + 1 }, (_, i) => i)
  const mediaName = new Map(project.media.map((m) => [m.id, m.name]))
  const mediaDur = new Map(project.media.map((m) => [m.id, m.duration]))

  function timeFromClientX(clientX: number): number {
    const el = boardRef.current
    if (!el) return 0
    const rect = el.getBoundingClientRect()
    const x = clientX - rect.left - LABEL_W + el.scrollLeft
    return Math.max(0, x / PX_PER_SEC)
  }

  function seekFromPointer(clientX: number) {
    onSeek(Math.min(timeFromClientX(clientX), axis))
  }

  function beginWindowDrag(onMove: (ev: PointerEvent) => void) {
    const up = () => {
      dragRef.current = null
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', up)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', up)
  }

  function onBoardPointerDown(e: ReactPointerEvent<HTMLDivElement>) {
    if (dragRef.current) return
    if ((e.target as HTMLElement).closest('.clip-block')) return
    seekFromPointer(e.clientX)
    beginWindowDrag((ev) => seekFromPointer(ev.clientX))
  }

  function onClipPointerDown(
    e: ReactPointerEvent<HTMLDivElement>,
    clip: Clip,
  ) {
    e.stopPropagation()
    e.preventDefault()
    setSelectedId(clip.id)

    const rect = e.currentTarget.getBoundingClientRect()
    const localX = e.clientX - rect.left
    const nearIn = localX <= EDGE_PX
    const nearOut = localX >= rect.width - EDGE_PX
    const alt = e.altKey

    if (nearIn || nearOut) {
      const edge: TrimEdge = nearIn ? 'in' : 'out'
      dragRef.current = {
        kind: alt ? 'stretch' : 'trim',
        clipId: clip.id,
        edge,
        mediaDuration: mediaDur.get(clip.mediaId),
      }
      beginWindowDrag((ev) => {
        const mode = dragRef.current
        if (!mode || (mode.kind !== 'trim' && mode.kind !== 'stretch')) return
        const t = timeFromClientX(ev.clientX)
        const p = projectRef.current
        const next =
          mode.kind === 'stretch'
            ? stretch(p, mode.clipId, mode.edge, t)
            : trim(p, mode.clipId, mode.edge, t, mode.mediaDuration)
        onChange(next)
      })
      return
    }

    dragRef.current = {
      kind: 'move',
      clipId: clip.id,
      originStart: clip.start,
      originX: e.clientX,
    }
    beginWindowDrag((ev) => {
      const mode = dragRef.current
      if (!mode || mode.kind !== 'move') return
      const dt = (ev.clientX - mode.originX) / PX_PER_SEC
      onChange(move(projectRef.current, mode.clipId, mode.originStart + dt))
    })
  }

  function onRazor() {
    onChange(razor(projectRef.current, currentTime))
  }

  return (
    <section className="timeline-panel">
      <div className="panel-label timeline-label-row">
        <span>时间轴</span>
        <button type="button" className="timeline-razor-btn" onClick={onRazor}>
          剪切
        </button>
        <span className="timeline-hint">拖块移动 · 拖边修剪 · Alt+拖边变速</span>
      </div>
      <div
        ref={boardRef}
        className="timeline-board"
        onPointerDown={onBoardPointerDown}
      >
        <div className="timeline-ruler" style={{ width: LABEL_W + laneWidth }}>
          <div className="timeline-corner" />
          <div className="timeline-ruler-scale" style={{ width: laneWidth }}>
            {ticks.map((t) => (
              <div key={t} className="tick" style={{ left: t * PX_PER_SEC }}>
                <span>{fmtTick(t)}</span>
              </div>
            ))}
          </div>
        </div>

        <div className="timeline-tracks" style={{ width: LABEL_W + laneWidth }}>
          {project.tracks.map((track) => (
            <div key={track.id} className={`timeline-row kind-${track.kind}`}>
              <div className="track-label">
                <span className="track-name">{track.name}</span>
                <span className="track-kind">{track.kind}</span>
              </div>
              <div className="track-lane" style={{ width: laneWidth }}>
                <div className="lane-grid" />
                {track.clips.map((clip) => {
                  const rate = clip.rate ?? 1
                  const name = mediaName.get(clip.mediaId) ?? 'clip'
                  return (
                    <div
                      key={clip.id}
                      className={
                        'clip-block' +
                        (selectedId === clip.id ? ' is-selected' : '')
                      }
                      style={{
                        left: clip.start * PX_PER_SEC,
                        width: Math.max(clip.duration * PX_PER_SEC, 4),
                      }}
                      title={`${name} · ${rate.toFixed(2)}x`}
                      onPointerDown={(e) => onClipPointerDown(e, clip)}
                    >
                      <span className="clip-edge clip-edge-in" />
                      <span className="clip-label">
                        {name}
                        {Math.abs(rate - 1) > 0.01 ? ` ${rate.toFixed(2)}x` : ''}
                      </span>
                      <span className="clip-edge clip-edge-out" />
                    </div>
                  )
                })}
              </div>
            </div>
          ))}
        </div>

        <div className="playhead" style={{ left: playheadX }} />
      </div>
    </section>
  )
}

function fmtTick(sec: number) {
  const m = Math.floor(sec / 60)
  const s = Math.floor(sec % 60)
  return `${m}:${String(s).padStart(2, '0')}`
}

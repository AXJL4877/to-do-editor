import { useEffect, useMemo, useRef, useState } from 'react'
import { addToLibrary } from '../../../core/media/addToLibrary'
import { projectDuration, type Project } from '../../../core/project'
import { addClip } from '../../../core/timeline/actions/addClip'
import { seek } from '../../../core/timeline/actions/seek'
import { resolvePreview } from '../../../core/timeline/resolvePreview'
import { mediaFileUrl, probeMedia, uploadMedia } from '../lib/mediaApi'
import { saveProject } from '../lib/projectStore'
import { ProgramMonitor } from './ProgramMonitor'
import { Timeline } from './Timeline'
import { Toolbar } from './Toolbar'

type Props = {
  project: Project
  projectName: string
  onChange: (project: Project) => void
  onBack: () => void
  onSave: (project: Project) => void
}

export function Editor({ project, projectName, onChange, onBack, onSave }: Props) {
  const [playing, setPlaying] = useState(false)
  const [playhead, setPlayhead] = useState(project.playhead)
  const [importing, setImporting] = useState(false)
  const videoRef = useRef<HTMLVideoElement>(null)
  const playheadRef = useRef(playhead)
  const projectRef = useRef(project)
  playheadRef.current = playhead
  projectRef.current = project

  const preview = useMemo(
    () => resolvePreview(project, playhead, { kinds: ['video'] }),
    [project, playhead],
  )
  const previewUrl =
    preview?.media.path ? mediaFileUrl(preview.media.path) : null
  const axisDuration = projectDuration(project)

  useEffect(() => {
    setPlayhead(project.playhead)
  }, [project.playhead])

  useEffect(() => {
    const v = videoRef.current
    if (!v || !preview) return
    if (Math.abs(v.playbackRate - preview.clip.rate) > 0.001) {
      v.playbackRate = preview.clip.rate
    }
    // 播放中由 video 自身推进，避免每帧强制 seek 造成画面跳动/像被裁切
    if (playing) return
    const target = preview.mediaTime
    if (Math.abs(v.currentTime - target) > 0.04) {
      v.currentTime = target
    }
  }, [preview, playing])

  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    if (playing && previewUrl) {
      void v.play().catch(() => setPlaying(false))
    } else {
      v.pause()
    }
  }, [playing, previewUrl, preview?.clip.id])

  useEffect(() => {
    if (!playing) return
    let raf = 0
    let last = performance.now()
    const tick = (now: number) => {
      const dt = (now - last) / 1000
      last = now
      const nextT = playheadRef.current + dt
      const end = projectDuration(projectRef.current)
      if (nextT >= end) {
        setPlaying(false)
        applySeek(end)
        return
      }
      setPlayhead(nextT)
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [playing])

  function commit(next: Project) {
    onChange(next)
  }

  function applySeek(time: number) {
    const next = seek(projectRef.current, time)
    setPlayhead(next.playhead)
    commit(next)
  }

  async function onImport(file: File | undefined) {
    if (!file || !file.type.startsWith('video/') || importing) return
    setImporting(true)
    try {
      const saved = await uploadMedia(projectName, file)
      const probed = await probeMedia(saved.path)
      if (!probed.ok) throw new Error(probed.error)

      const item = {
        id: saved.id,
        name: saved.name,
        path: saved.path,
        duration: probed.duration,
        width: probed.width,
        height: probed.height,
      }
      let next = addToLibrary(project, item)
      const videoTrack =
        next.tracks.find((t) => t.kind === 'video') ?? next.tracks[0]
      let previewAt = 0
      if (videoTrack) {
        const start = videoTrack.clips.reduce(
          (max, c) => Math.max(max, c.start + c.duration),
          0,
        )
        next = addClip(next, videoTrack.id, {
          id: crypto.randomUUID(),
          mediaId: item.id,
          start,
          duration: item.duration,
          offset: 0,
          rate: 1,
        })
        next = {
          ...next,
          duration: Math.max(next.duration ?? 0, start + item.duration),
        }
        previewAt = start
        next = seek(next, previewAt)
      }
      setPlayhead(previewAt)
      commit(next)
      await saveProject(projectName, next)
      setPlaying(false)
    } catch (e) {
      console.error(e)
    } finally {
      setImporting(false)
    }
  }

  function togglePlay() {
    if (!previewUrl && !playing) return
    setPlaying((p) => !p)
  }

  function onSeek(time: number) {
    setPlaying(false)
    applySeek(time)
  }

  function handleSave() {
    const next = seek(project, playhead)
    commit(next)
    onSave(next)
  }

  return (
    <div className="app">
      <Toolbar
        projectName={projectName}
        clipName={preview?.media.name ?? null}
        currentTime={playhead}
        duration={axisDuration}
        playing={playing}
        canPlay={!!previewUrl && !importing}
        onImport={(f) => void onImport(f)}
        onTogglePlay={togglePlay}
        onBack={onBack}
        onSave={handleSave}
      />
      <ProgramMonitor
        src={previewUrl}
        videoRef={videoRef}
        width={preview?.media.width}
        height={preview?.media.height}
        onLoadedMetadata={() => {
          const v = videoRef.current
          const hit = resolvePreview(projectRef.current, playheadRef.current, {
            kinds: ['video'],
          })
          if (v && hit) v.currentTime = hit.mediaTime
        }}
        onTimeUpdate={() => undefined}
        onPlay={() => setPlaying(true)}
        onPause={() => {
          setPlaying(false)
          applySeek(playheadRef.current)
        }}
        onEnded={() => setPlaying(false)}
      />
      <Timeline
        project={project}
        currentTime={playhead}
        onSeek={onSeek}
        onChange={(next) => commit(next)}
      />
    </div>
  )
}

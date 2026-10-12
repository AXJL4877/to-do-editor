import type { MediaItem } from '../media/types'
import type { Project } from '../project'
import { clipEnd, type Clip } from './clip'
import type { Track, TrackKind } from './track'

export type PreviewHit = {
  trackId: string
  trackKind: TrackKind
  clip: Clip
  media: MediaItem
  /** 时间线时间（秒） */
  timelineTime: number
  /** 素材内时间（秒）：offset + (t - start) * rate */
  mediaTime: number
  /** 相对本段起点的本地时间（秒） */
  localTime: number
}

/**
 * 播放头落在哪段 clip，以及对应的素材内时间。
 * 默认优先视频轨（自上而下第一条命中）；可指定 trackId / kinds。
 */
export function resolvePreview(
  project: Project,
  timelineTime: number,
  opts?: {
    trackId?: string
    kinds?: TrackKind[]
  },
): PreviewHit | null {
  const t = Math.max(timelineTime, 0)
  const kinds = opts?.kinds ?? (['video', 'audio', 'sfx'] as TrackKind[])
  const tracks = project.tracks.filter((track) => {
    if (opts?.trackId) return track.id === opts.trackId
    return kinds.includes(track.kind)
  })

  for (const track of tracks) {
    const hit = hitOnTrack(project, track, t)
    if (hit) return hit
  }
  return null
}

function hitOnTrack(
  project: Project,
  track: Track,
  t: number,
): PreviewHit | null {
  for (const clip of track.clips) {
    if (t < clip.start || t >= clipEnd(clip)) continue
    const media = project.media.find((m) => m.id === clip.mediaId)
    if (!media) continue
    const localTime = t - clip.start
    const mediaTime = clip.offset + localTime * clip.rate
    return {
      trackId: track.id,
      trackKind: track.kind,
      clip,
      media,
      timelineTime: t,
      mediaTime: clampMediaTime(mediaTime, media.duration),
      localTime,
    }
  }
  return null
}

function clampMediaTime(time: number, duration: number): number {
  if (!Number.isFinite(duration) || duration <= 0) return Math.max(time, 0)
  return Math.min(Math.max(time, 0), Math.max(duration - 1e-4, 0))
}

import type { Project } from '../../project'
import { MIN_CLIP_DURATION, normalizeClip, type Clip } from '../clip'
import { mapTracks } from './mutate'

function splitClip(clip: Clip, time: number, newId: string): [Clip, Clip] | null {
  const local = time - clip.start
  if (local <= MIN_CLIP_DURATION || local >= clip.duration - MIN_CLIP_DURATION) {
    return null
  }
  const left = normalizeClip({
    ...clip,
    duration: local,
  })
  const right = normalizeClip({
    ...clip,
    id: newId,
    start: time,
    duration: clip.duration - local,
    offset: clip.offset + local * clip.rate,
    rate: clip.rate,
  })
  return [left, right]
}

/** 在指定时间切开轨道上的 clip；不传 trackId 则对所有轨执行。 */
export function razor(
  project: Project,
  time: number,
  trackId?: string,
  newId: () => string = () =>
    `c_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`,
): Project {
  return mapTracks(project, (track) => {
    if (trackId && track.id !== trackId) return track
    const nextClips: Clip[] = []
    for (const clip of track.clips) {
      const hit = time > clip.start && time < clip.start + clip.duration
      if (!hit) {
        nextClips.push(clip)
        continue
      }
      const parts = splitClip(clip, time, newId())
      if (!parts) {
        nextClips.push(clip)
        continue
      }
      nextClips.push(parts[0], parts[1])
    }
    return { ...track, clips: nextClips }
  })
}

import type { Project } from '../../project'
import {
  MIN_CLIP_DURATION,
  normalizeClip,
  type Clip,
} from '../clip'
import { patchClip } from './mutate'

export type TrimEdge = 'in' | 'out'

/**
 * 前后端修剪（不改变 rate）。
 * in：拖左缘 → 改 start / offset / duration
 * out：拖右缘 → 改 duration（可露出更多素材）
 * mediaDuration 有值时限制不超出素材。
 */
export function trim(
  project: Project,
  clipId: string,
  edge: TrimEdge,
  time: number,
  mediaDuration?: number,
): Project {
  return patchClip(project, clipId, (clip, trackId) => {
    const next =
      edge === 'in'
        ? trimIn(clip, time, mediaDuration)
        : trimOut(clip, time, mediaDuration)
    return { clip: next, trackId }
  })
}

function trimIn(clip: Clip, time: number, mediaDuration?: number): Clip {
  const end = clip.start + clip.duration
  let start = Math.min(Math.max(time, 0), end - MIN_CLIP_DURATION)
  let delta = start - clip.start
  let offset = clip.offset + delta * clip.rate
  let duration = end - start

  if (offset < 0) {
    const fix = -offset / clip.rate
    start += fix
    offset = 0
    duration = end - start
  }

  if (typeof mediaDuration === 'number') {
    const maxSpan = mediaDuration - offset
    if (duration * clip.rate > maxSpan) {
      duration = Math.max(maxSpan / clip.rate, MIN_CLIP_DURATION)
    }
  }

  return normalizeClip({ ...clip, start, offset, duration })
}

function trimOut(clip: Clip, time: number, mediaDuration?: number): Clip {
  let duration = Math.max(time - clip.start, MIN_CLIP_DURATION)
  if (typeof mediaDuration === 'number') {
    const maxSpan = Math.max(mediaDuration - clip.offset, MIN_CLIP_DURATION * clip.rate)
    duration = Math.min(duration, maxSpan / clip.rate)
  }
  return normalizeClip({ ...clip, duration })
}

import type { Project } from '../../project'
import {
  MAX_CLIP_RATE,
  MIN_CLIP_DURATION,
  MIN_CLIP_RATE,
  normalizeClip,
  sourceSpan,
  type Clip,
} from '../clip'
import type { TrimEdge } from './trim'
import { patchClip } from './mutate'

/**
 * 拖边变速：保持素材区间（sourceSpan）不变，改 timeline duration → 改 rate。
 * 拉长 = 减速；缩短 = 加速。
 */
export function stretch(
  project: Project,
  clipId: string,
  edge: TrimEdge,
  time: number,
): Project {
  return patchClip(project, clipId, (clip, trackId) => ({
    clip: edge === 'in' ? stretchIn(clip, time) : stretchOut(clip, time),
    trackId,
  }))
}

function stretchOut(clip: Clip, time: number): Clip {
  const span = sourceSpan(clip)
  let duration = Math.max(time - clip.start, MIN_CLIP_DURATION)
  let rate = span / duration
  if (rate < MIN_CLIP_RATE) {
    rate = MIN_CLIP_RATE
    duration = span / rate
  } else if (rate > MAX_CLIP_RATE) {
    rate = MAX_CLIP_RATE
    duration = span / rate
  }
  return normalizeClip({ ...clip, duration, rate })
}

function stretchIn(clip: Clip, time: number): Clip {
  const end = clip.start + clip.duration
  const span = sourceSpan(clip)
  let start = Math.min(Math.max(time, 0), end - MIN_CLIP_DURATION)
  let duration = end - start
  let rate = span / duration
  if (rate < MIN_CLIP_RATE) {
    rate = MIN_CLIP_RATE
    duration = span / rate
    start = end - duration
  } else if (rate > MAX_CLIP_RATE) {
    rate = MAX_CLIP_RATE
    duration = span / rate
    start = end - duration
  }
  return normalizeClip({ ...clip, start: Math.max(start, 0), duration, rate })
}

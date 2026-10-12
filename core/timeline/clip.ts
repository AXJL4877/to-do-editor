export type Clip = {
  id: string
  mediaId: string
  /** 时间线上的起点（秒） */
  start: number
  /** 在时间线上占用的时长（秒） */
  duration: number
  /** 素材入点（秒） */
  offset: number
  /** 播放速率：1=原速；>1 加速；<1 减速。素材用量 = duration * rate */
  rate: number
}

export const MIN_CLIP_DURATION = 0.05
export const MIN_CLIP_RATE = 0.1
export const MAX_CLIP_RATE = 8

export function normalizeClip(partial: Partial<Clip> & Pick<Clip, 'id' | 'mediaId'>): Clip {
  const duration = Math.max(partial.duration ?? MIN_CLIP_DURATION, MIN_CLIP_DURATION)
  const rate = clamp(partial.rate ?? 1, MIN_CLIP_RATE, MAX_CLIP_RATE)
  return {
    id: partial.id,
    mediaId: partial.mediaId,
    start: Math.max(partial.start ?? 0, 0),
    duration,
    offset: Math.max(partial.offset ?? 0, 0),
    rate,
  }
}

/** 该 clip 消耗的素材时长 */
export function sourceSpan(clip: Clip): number {
  return clip.duration * clip.rate
}

export function clipEnd(clip: Clip): number {
  return clip.start + clip.duration
}

function clamp(n: number, min: number, max: number) {
  return Math.min(Math.max(n, min), max)
}

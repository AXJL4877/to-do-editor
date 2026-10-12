import type { Clip } from './clip'

export type { Clip } from './clip'

export type TrackKind = 'video' | 'audio' | 'sfx'

export type Track = {
  id: string
  name: string
  kind: TrackKind
  muted?: boolean
  locked?: boolean
  clips: Clip[]
}

export const TRACK_DEFAULTS = {
  muted: false,
  locked: false,
  clips: [] as Clip[],
} as const

export function createDefaultTracks(): Track[] {
  return [
    { id: 'v1', name: '视频1', kind: 'video', ...TRACK_DEFAULTS, clips: [] },
    { id: 'a1', name: '对白', kind: 'audio', ...TRACK_DEFAULTS, clips: [] },
    { id: 'sfx1', name: '音效', kind: 'sfx', ...TRACK_DEFAULTS, clips: [] },
  ]
}

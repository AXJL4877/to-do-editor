import type { MediaItem } from './media/types'
import { normalizeClip } from './timeline/clip'
import { createDefaultTracks, type Track } from './timeline/track'

export type { MediaItem } from './media/types'

export type Project = {
  version: 1
  fps: number
  playhead: number
  duration?: number
  media: MediaItem[]
  tracks: Track[]
}

function normalizeTracks(tracks: Track[]): Track[] {
  return tracks.map((track) => ({
    ...track,
    clips: (track.clips ?? []).map((clip) => normalizeClip(clip)),
  }))
}

export function createProject(partial?: Partial<Omit<Project, 'version'>>): Project {
  return {
    version: 1,
    fps: partial?.fps ?? 30,
    playhead: partial?.playhead ?? 0,
    duration: partial?.duration,
    media: partial?.media ?? [],
    tracks: normalizeTracks(partial?.tracks ?? createDefaultTracks()),
  }
}

export function projectDuration(project: Project): number {
  let max = typeof project.duration === 'number' && project.duration > 0 ? project.duration : 0
  for (const m of project.media) {
    if (typeof m.duration === 'number' && m.duration > max) max = m.duration
  }
  for (const track of project.tracks) {
    for (const clip of track.clips) {
      const end = clip.start + clip.duration
      if (end > max) max = end
    }
  }
  return max > 0 ? max : 10
}

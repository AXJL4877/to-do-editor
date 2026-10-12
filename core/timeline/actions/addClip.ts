import type { Project } from '../../project'
import { normalizeClip, type Clip } from '../clip'

/** 把已有媒体以 clip 形式放到指定轨道（只改 tracks）。 */
export function addClip(project: Project, trackId: string, clip: Clip): Project {
  const exists = project.media.some((m) => m.id === clip.mediaId)
  if (!exists) return project
  const next = normalizeClip(clip)

  return {
    ...project,
    tracks: project.tracks.map((track) => {
      if (track.id !== trackId) return track
      if (track.clips.some((c) => c.id === next.id)) return track
      return { ...track, clips: [...track.clips, next] }
    }),
  }
}

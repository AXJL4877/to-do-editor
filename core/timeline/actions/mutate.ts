import type { Project } from '../../project'
import type { Clip } from '../clip'
import type { Track } from '../track'

export function findClip(
  project: Project,
  clipId: string,
): { track: Track; trackId: string; clip: Clip; index: number } | null {
  for (const track of project.tracks) {
    const index = track.clips.findIndex((c) => c.id === clipId)
    if (index >= 0) {
      return { track, trackId: track.id, clip: track.clips[index], index }
    }
  }
  return null
}

export function mapTracks(
  project: Project,
  mapTrack: (track: Track) => Track,
): Project {
  return {
    ...project,
    tracks: project.tracks.map(mapTrack),
  }
}

/** 替换 clip；可选换到另一轨。update 返回 null 表示删除。 */
export function patchClip(
  project: Project,
  clipId: string,
  update: (clip: Clip, trackId: string) => { clip: Clip; trackId: string } | null,
): Project {
  const found = findClip(project, clipId)
  if (!found) return project
  const next = update(found.clip, found.trackId)
  if (!next) {
    return mapTracks(project, (track) =>
      track.id === found.trackId
        ? { ...track, clips: track.clips.filter((c) => c.id !== clipId) }
        : track,
    )
  }

  if (next.trackId === found.trackId) {
    return mapTracks(project, (track) => {
      if (track.id !== found.trackId) return track
      return {
        ...track,
        clips: track.clips.map((c) => (c.id === clipId ? next.clip : c)),
      }
    })
  }

  return mapTracks(project, (track) => {
    if (track.id === found.trackId) {
      return { ...track, clips: track.clips.filter((c) => c.id !== clipId) }
    }
    if (track.id === next.trackId) {
      if (track.clips.some((c) => c.id === next.clip.id)) return track
      return { ...track, clips: [...track.clips, next.clip] }
    }
    return track
  })
}

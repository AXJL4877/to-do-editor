import type { Project } from '../../project'
import { normalizeClip } from '../clip'
import { patchClip } from './mutate'

/** 移动 clip 起点；可选换轨。 */
export function move(
  project: Project,
  clipId: string,
  start: number,
  trackId?: string,
): Project {
  return patchClip(project, clipId, (clip, fromTrackId) => ({
    clip: normalizeClip({ ...clip, start: Math.max(0, start) }),
    trackId: trackId ?? fromTrackId,
  }))
}

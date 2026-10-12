import type { Project } from '../project'
import type { MediaItem } from './types'

/** 往工程 media[] 登记一条素材（不碰 ffmpeg / 磁盘）。 */
export function addToLibrary(project: Project, item: MediaItem): Project {
  if (project.media.some((m) => m.id === item.id)) return project
  return {
    ...project,
    media: [...project.media, item],
  }
}

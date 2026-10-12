import { projectDuration, type Project } from '../../project'

/** 改 playhead（只动播放头，不碰 tracks/clips）。 */
export function seek(project: Project, time: number): Project {
  const end = projectDuration(project)
  const playhead = Math.min(Math.max(time, 0), end)
  if (playhead === project.playhead) return project
  return { ...project, playhead }
}

import fs from 'node:fs'
import { createProject, type Project } from '../../../../core/project.ts'
import { projectDir, projectMediaDir } from '../../path.ts'
import { saveProject } from './save.ts'

export function createNewProject(
  name: string,
  partial?: Partial<Omit<Project, 'version'>>,
): Project {
  fs.mkdirSync(projectDir(name), { recursive: true })
  fs.mkdirSync(projectMediaDir(name), { recursive: true })
  const project = createProject({
    playhead: 0,
    ...partial,
    media: partial?.media ?? [],
  })
  saveProject(name, project)
  return project
}

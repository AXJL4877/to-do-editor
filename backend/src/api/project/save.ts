import fs from 'node:fs'
import type { Project } from '../../../../core/project.ts'
import { projectDir, projectJsonPath, projectMediaDir } from '../../path.ts'

export function saveProject(name: string, project: Project): void {
  fs.mkdirSync(projectDir(name), { recursive: true })
  fs.mkdirSync(projectMediaDir(name), { recursive: true })
  fs.writeFileSync(projectJsonPath(name), JSON.stringify(project, null, 2), 'utf8')
}

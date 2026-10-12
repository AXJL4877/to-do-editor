import fs from 'node:fs'
import { createProject, type Project } from '../../../../core/project.ts'
import { projectJsonPath } from '../../path.ts'

export function loadProject(name: string): Project | null {
  const full = projectJsonPath(name)
  if (!fs.existsSync(full)) return null
  const raw = JSON.parse(fs.readFileSync(full, 'utf8')) as Project
  if (raw.version !== 1) return null
  return createProject(raw)
}

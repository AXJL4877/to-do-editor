import fs from 'node:fs'
import { DATA_DIR, projectJsonPath } from '../../path.ts'

export function listProjects(): string[] {
  if (!fs.existsSync(DATA_DIR)) return []
  return fs
    .readdirSync(DATA_DIR, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .filter((name) => fs.existsSync(projectJsonPath(name)))
    .sort((a, b) => a.localeCompare(b))
}

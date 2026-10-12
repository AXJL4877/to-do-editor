import fs from 'node:fs'
import path from 'node:path'
import { DATA_DIR, projectDir } from '../../path.ts'

/** 删除整个工程目录 data/<name>/（含 project.json 与 media/）。 */
export function deleteProject(name: string): boolean {
  const dir = projectDir(name)
  const root = path.normalize(DATA_DIR + path.sep)
  const resolved = path.normalize(dir)
  if (!resolved.toLowerCase().startsWith(root.toLowerCase())) {
    throw new Error('path outside data')
  }
  if (!fs.existsSync(resolved)) return false
  fs.rmSync(resolved, { recursive: true, force: true })
  return true
}

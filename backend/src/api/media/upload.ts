import { randomUUID } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { projectMediaDir, REPO_ROOT, sanitizeProjectName } from '../../path.ts'

export function saveMediaFile(
  projectName: string,
  originalName: string,
  data: Buffer,
): {
  id: string
  name: string
  path: string
} {
  const project = sanitizeProjectName(projectName)
  const id = randomUUID()
  const safe = originalName.replace(/[\\/]/g, '_').trim() || 'media.bin'
  const mediaDir = projectMediaDir(project)
  fs.mkdirSync(mediaDir, { recursive: true })
  const fileName = `${id}_${safe}`
  const full = path.join(mediaDir, fileName)
  fs.writeFileSync(full, data)
  return {
    id,
    name: safe,
    path: path.relative(REPO_ROOT, full).replace(/\\/g, '/'),
  }
}

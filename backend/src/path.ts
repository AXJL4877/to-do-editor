import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))

export const BACKEND_ROOT = path.resolve(here, '..')
export const REPO_ROOT = path.resolve(BACKEND_ROOT, '..')
export const DATA_DIR = path.join(REPO_ROOT, 'data')
export const FFMPEG_DIR = path.join(BACKEND_ROOT, 'ffmpeg')
export const FFMPEG_BIN = path.join(FFMPEG_DIR, 'bin', 'ffmpeg.exe')
export const FFPROBE_BIN = path.join(FFMPEG_DIR, 'bin', 'ffprobe.exe')

export function sanitizeProjectName(name: string): string {
  const base = name.replace(/[\\/]/g, '').trim()
  if (!base || base === '.' || base === '..') {
    throw new Error('invalid project name')
  }
  return base.replace(/\.json$/i, '')
}

export function projectDir(name: string): string {
  return path.join(DATA_DIR, sanitizeProjectName(name))
}

export function projectJsonPath(name: string): string {
  return path.join(projectDir(name), 'project.json')
}

export function projectMediaDir(name: string): string {
  return path.join(projectDir(name), 'media')
}

export function resolveMediaPath(filePath: string): string {
  const resolved = path.isAbsolute(filePath)
    ? path.normalize(filePath)
    : path.normalize(path.join(REPO_ROOT, filePath))
  const root = path.normalize(REPO_ROOT + path.sep)
  if (!resolved.toLowerCase().startsWith(root.toLowerCase())) {
    throw new Error('path outside repo')
  }
  return resolved
}

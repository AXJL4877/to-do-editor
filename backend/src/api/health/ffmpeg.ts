import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { FFMPEG_BIN, FFPROBE_BIN } from '../../path.ts'

function versionOf(bin: string): { ok: boolean; path: string; version: string; error?: string } {
  if (!existsSync(bin)) {
    return { ok: false, path: bin, version: '', error: 'file not found' }
  }
  const r = spawnSync(bin, ['-version'], {
    encoding: 'utf8',
    windowsHide: true,
  })
  if (r.error) {
    return { ok: false, path: bin, version: '', error: r.error.message }
  }
  const out = `${r.stdout || ''}${r.stderr || ''}`.trim()
  const first = out.split(/\r?\n/).find((l) => l.trim()) || ''
  return {
    ok: r.status === 0 && !!first,
    path: bin,
    version: first,
    error: r.status === 0 ? undefined : `exit ${r.status}`,
  }
}

export function ffmpegHealth() {
  const ffmpeg = versionOf(FFMPEG_BIN)
  const ffprobe = versionOf(FFPROBE_BIN)
  return {
    ok: ffmpeg.ok && ffprobe.ok,
    ffmpeg,
    ffprobe,
  }
}

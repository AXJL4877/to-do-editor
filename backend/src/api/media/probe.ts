import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { FFPROBE_BIN, resolveMediaPath } from '../../path.ts'

export type ProbeResult = {
  ok: true
  path: string
  duration: number
  width?: number
  height?: number
}

export type ProbeError = {
  ok: false
  path: string
  error: string
}

export function probeMedia(filePath: string): ProbeResult | ProbeError {
  let full: string
  try {
    full = resolveMediaPath(filePath)
  } catch (e) {
    return { ok: false, path: filePath, error: e instanceof Error ? e.message : String(e) }
  }

  if (!existsSync(FFPROBE_BIN)) {
    return { ok: false, path: full, error: `ffprobe not found: ${FFPROBE_BIN}` }
  }
  if (!existsSync(full)) {
    return { ok: false, path: full, error: 'file not found' }
  }

  const r = spawnSync(
    FFPROBE_BIN,
    [
      '-v',
      'quiet',
      '-print_format',
      'json',
      '-show_format',
      '-show_streams',
      full,
    ],
    { encoding: 'utf8', windowsHide: true },
  )

  if (r.error) {
    return { ok: false, path: full, error: r.error.message }
  }
  if (r.status !== 0) {
    return {
      ok: false,
      path: full,
      error: (r.stderr || r.stdout || `exit ${r.status}`).trim(),
    }
  }

  try {
    const json = JSON.parse(r.stdout || '{}') as {
      format?: { duration?: string }
      streams?: Array<{ codec_type?: string; width?: number; height?: number }>
    }
    const duration = Number(json.format?.duration ?? NaN)
    if (!Number.isFinite(duration) || duration <= 0) {
      return { ok: false, path: full, error: 'duration missing' }
    }
    const video = json.streams?.find((s) => s.codec_type === 'video')
    return {
      ok: true,
      path: full,
      duration,
      width: video?.width,
      height: video?.height,
    }
  } catch (e) {
    return { ok: false, path: full, error: e instanceof Error ? e.message : String(e) }
  }
}

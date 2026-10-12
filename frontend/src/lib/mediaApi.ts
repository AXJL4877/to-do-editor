export type UploadResult = {
  id: string
  name: string
  path: string
}

export type ProbeResult =
  | { ok: true; path: string; duration: number; width?: number; height?: number }
  | { ok: false; path: string; error: string }

export async function uploadMedia(projectName: string, file: File): Promise<UploadResult> {
  const res = await fetch(`/api/media/upload?project=${encodeURIComponent(projectName)}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/octet-stream',
      'x-filename': encodeURIComponent(file.name),
      'x-project': projectName,
    },
    body: file,
  })
  if (!res.ok) throw new Error(`upload failed: ${res.status}`)
  return (await res.json()) as UploadResult
}

export async function probeMedia(filePath: string): Promise<ProbeResult> {
  const res = await fetch('/api/media/probe', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path: filePath }),
  })
  return (await res.json()) as ProbeResult
}

/** 给 <video> 用的可播放地址（走后端读 data/ 下文件） */
export function mediaFileUrl(relativePath: string): string {
  return `/api/media/file?path=${encodeURIComponent(relativePath)}`
}

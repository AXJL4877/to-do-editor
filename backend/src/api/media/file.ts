import fs from 'node:fs'
import path from 'node:path'
import type http from 'node:http'
import { resolveMediaPath } from '../../path.ts'

const MIME: Record<string, string> = {
  '.mp4': 'video/mp4',
  '.webm': 'video/webm',
  '.mov': 'video/quicktime',
  '.mkv': 'video/x-matroska',
  '.wav': 'audio/wav',
  '.mp3': 'audio/mpeg',
  '.aac': 'audio/aac',
  '.m4a': 'audio/mp4',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
}

/** 把工程内媒体文件流式返回给前端 <video>/<audio> */
export function sendMediaFile(
  req: http.IncomingMessage,
  res: http.ServerResponse,
  relativePath: string,
): void {
  const full = resolveMediaPath(relativePath)
  if (!fs.existsSync(full) || !fs.statSync(full).isFile()) {
    res.writeHead(404, {
      'content-type': 'application/json; charset=utf-8',
      'access-control-allow-origin': '*',
    })
    res.end(JSON.stringify({ error: 'file not found' }))
    return
  }

  const ext = path.extname(full).toLowerCase()
  const type = MIME[ext] || 'application/octet-stream'
  const stat = fs.statSync(full)
  const size = stat.size
  const range = req.headers.range

  if (range) {
    const m = /^bytes=(\d*)-(\d*)$/.exec(range)
    if (m) {
      const start = m[1] ? Number(m[1]) : 0
      const end = m[2] ? Number(m[2]) : size - 1
      if (start >= size || end >= size || start > end) {
        res.writeHead(416, {
          'content-range': `bytes */${size}`,
          'access-control-allow-origin': '*',
        })
        res.end()
        return
      }
      res.writeHead(206, {
        'content-type': type,
        'content-length': end - start + 1,
        'content-range': `bytes ${start}-${end}/${size}`,
        'accept-ranges': 'bytes',
        'access-control-allow-origin': '*',
      })
      fs.createReadStream(full, { start, end }).pipe(res)
      return
    }
  }

  res.writeHead(200, {
    'content-type': type,
    'content-length': size,
    'accept-ranges': 'bytes',
    'access-control-allow-origin': '*',
  })
  fs.createReadStream(full).pipe(res)
}

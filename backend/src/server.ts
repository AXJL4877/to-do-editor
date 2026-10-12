import http from 'node:http'
import { ffmpegHealth } from './api/health/ffmpeg.ts'
import { sendMediaFile } from './api/media/file.ts'
import { probeMedia } from './api/media/probe.ts'
import { saveMediaFile } from './api/media/upload.ts'
import {
  createNewProject,
  deleteProject,
  listProjects,
  loadProject,
  saveProject,
} from './api/project/index.ts'
import type { Project } from '../../core/project.ts'

const PORT = Number(process.env.PORT || 8787)

function sendJson(res: http.ServerResponse, status: number, body: unknown) {
  res.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'access-control-allow-origin': '*',
    'access-control-allow-headers': '*',
    'access-control-allow-methods': 'GET,POST,PUT,DELETE,OPTIONS',
  })
  res.end(JSON.stringify(body))
}

function readBody(req: http.IncomingMessage): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    req.on('data', (c) => chunks.push(c))
    req.on('end', () => resolve(Buffer.concat(chunks)))
    req.on('error', reject)
  })
}

function projectNameFromUrl(url: string): string | null {
  const m = url.match(/^\/api\/projects\/([^/?#]+)/)
  if (!m) return null
  try {
    return decodeURIComponent(m[1])
  } catch {
    return null
  }
}

const server = http.createServer(async (req, res) => {
  const method = req.method || 'GET'
  const url = req.url || '/'

  if (method === 'OPTIONS') {
    res.writeHead(204, {
      'access-control-allow-origin': '*',
      'access-control-allow-headers': '*',
      'access-control-allow-methods': 'GET,POST,PUT,DELETE,OPTIONS',
    })
    res.end()
    return
  }

  try {
    if (method === 'GET' && (url === '/api/health/ffmpeg' || url.startsWith('/api/health/ffmpeg?'))) {
      const health = ffmpegHealth()
      sendJson(res, health.ok ? 200 : 500, health)
      return
    }

    if (method === 'GET' && (url === '/api/media/file' || url.startsWith('/api/media/file?'))) {
      const q = new URL(url, 'http://127.0.0.1')
      const filePath = q.searchParams.get('path')
      if (!filePath) {
        sendJson(res, 400, { error: 'path required' })
        return
      }
      sendMediaFile(req, res, filePath)
      return
    }

    if (method === 'POST' && (url === '/api/media/probe' || url.startsWith('/api/media/probe?'))) {
      const raw = (await readBody(req)).toString('utf8')
      const body = JSON.parse(raw || '{}') as { path?: string }
      if (!body.path) {
        sendJson(res, 400, { ok: false, error: 'path required' })
        return
      }
      const result = probeMedia(body.path)
      sendJson(res, result.ok ? 200 : 400, result)
      return
    }

    if (method === 'POST' && (url === '/api/media/upload' || url.startsWith('/api/media/upload?'))) {
      const q = new URL(url, 'http://127.0.0.1')
      const project =
        q.searchParams.get('project') ||
        (typeof req.headers['x-project'] === 'string' ? req.headers['x-project'] : '')
      if (!project) {
        sendJson(res, 400, { error: 'project required' })
        return
      }
      const filename =
        (typeof req.headers['x-filename'] === 'string' && req.headers['x-filename']) ||
        'media.bin'
      const data = await readBody(req)
      if (!data.length) {
        sendJson(res, 400, { error: 'empty body' })
        return
      }
      const saved = saveMediaFile(project, decodeURIComponent(filename), data)
      sendJson(res, 201, saved)
      return
    }

    if (method === 'GET' && (url === '/api/projects' || url === '/api/projects/')) {
      sendJson(res, 200, { names: listProjects() })
      return
    }

    if (method === 'POST' && (url === '/api/projects' || url === '/api/projects/')) {
      const raw = (await readBody(req)).toString('utf8')
      const body = (raw ? JSON.parse(raw) : {}) as Partial<Project> & { name?: string }
      const projectName = typeof body.name === 'string' ? body.name.trim() : ''
      if (!projectName) {
        sendJson(res, 400, { error: 'name required' })
        return
      }
      const { name: _ignored, ...partial } = body
      const project = createNewProject(projectName, partial)
      sendJson(res, 201, { name: projectName, project })
      return
    }

    const name = projectNameFromUrl(url)
    if (name) {
      if (method === 'GET') {
        const project = loadProject(name)
        if (!project) {
          sendJson(res, 404, { error: 'not found' })
          return
        }
        sendJson(res, 200, project)
        return
      }

      if (method === 'PUT') {
        const raw = (await readBody(req)).toString('utf8')
        const data = JSON.parse(raw) as Project
        saveProject(name, data)
        res.writeHead(204, { 'access-control-allow-origin': '*' })
        res.end()
        return
      }

      if (method === 'POST') {
        const raw = (await readBody(req)).toString('utf8')
        const partial = raw ? (JSON.parse(raw) as Partial<Project>) : undefined
        const project = createNewProject(name, partial)
        sendJson(res, 201, { name, project })
        return
      }

      if (method === 'DELETE') {
        const ok = deleteProject(name)
        if (!ok) {
          sendJson(res, 404, { error: 'not found' })
          return
        }
        res.writeHead(204, { 'access-control-allow-origin': '*' })
        res.end()
        return
      }
    }

    sendJson(res, 404, { error: 'not found' })
  } catch (e) {
    sendJson(res, 400, { error: e instanceof Error ? e.message : String(e) })
  }
})

server.listen(PORT, '127.0.0.1', () => {
  process.stdout.write(`project api http://127.0.0.1:${PORT}\n`)
})

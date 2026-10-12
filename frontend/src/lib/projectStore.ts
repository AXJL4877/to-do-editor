import type { Project } from '../../../core/project'

async function readJson<T>(url: string): Promise<T | null> {
  const res = await fetch(url)
  if (!res.ok) return null
  return (await res.json()) as T
}

export async function listProjects(): Promise<string[]> {
  const data = await readJson<{ names: string[] }>('/api/projects')
  return data?.names ?? []
}

export async function createProjectRemote(name: string): Promise<Project | null> {
  const res = await fetch('/api/projects', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name }),
  })
  if (!res.ok) return null
  const data = (await res.json()) as { project: Project }
  return data.project ?? null
}

export async function loadProject(name: string): Promise<Project | null> {
  return readJson<Project>(`/api/projects/${encodeURIComponent(name)}`)
}

export async function saveProject(name: string, project: Project): Promise<boolean> {
  const res = await fetch(`/api/projects/${encodeURIComponent(name)}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(project, null, 2),
  })
  return res.ok
}

export async function deleteProject(name: string): Promise<boolean> {
  const res = await fetch(`/api/projects/${encodeURIComponent(name)}`, {
    method: 'DELETE',
  })
  return res.ok
}

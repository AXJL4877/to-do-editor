import { useEffect, useRef, useState } from 'react'
import { createProject, type Project } from '../../../core/project'
import {
  createProjectRemote,
  deleteProject,
  listProjects,
  loadProject,
  saveProject,
} from '../lib/projectStore'
import { AppButton } from './AppButton'

type Props = {
  onOpen: (project: Project, name: string) => void
}

export function ProjectWindow({ onOpen }: Props) {
  const [recent, setRecent] = useState<string[]>([])
  const [name, setName] = useState('untitled')
  const [busy, setBusy] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)

  async function refresh() {
    setRecent(await listProjects())
  }

  useEffect(() => {
    void refresh()
  }, [])

  async function startNew() {
    const projectName = name.trim() || 'untitled'
    setBusy(true)
    const project = await createProjectRemote(projectName)
    setBusy(false)
    if (!project) return
    await refresh()
    onOpen(createProject(project), projectName)
  }

  async function openSaved(projectName: string) {
    setBusy(true)
    const project = await loadProject(projectName)
    setBusy(false)
    if (!project || project.version !== 1) return
    onOpen(createProject(project), projectName)
  }

  async function removeSaved(projectName: string) {
    if (!window.confirm(`删除工程「${projectName}」？将移除 data/${projectName}/ 全部内容。`)) {
      return
    }
    setBusy(true)
    await deleteProject(projectName)
    setBusy(false)
    await refresh()
  }

  function openFile(file: File | undefined) {
    if (!file) return
    const reader = new FileReader()
    reader.onload = async () => {
      try {
        const data = JSON.parse(String(reader.result)) as Project
        if (data.version !== 1) return
        const projectName = file.name.replace(/\.json$/i, '') || 'untitled'
        const project = createProject(data)
        await saveProject(projectName, project)
        await refresh()
        onOpen(project, projectName)
      } catch {
        /* ignore */
      }
    }
    reader.readAsText(file)
  }

  return (
    <div className="project-shell">
      <div className="project-card">
        <div className="project-inner">
          <div className="project-brand">TO DO</div>
          <h1 className="project-title">工程</h1>
          <p className="project-sub">新建或打开工程，数据保存在 data/</p>

          <label className="project-field">
            <span>工程名</span>
            <input
              className="project-input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              spellCheck={false}
            />
          </label>

          <div className="project-actions">
            <AppButton variant="primary" onClick={() => void startNew()} disabled={busy}>
              新建工程
            </AppButton>
            <AppButton onClick={() => fileRef.current?.click()} disabled={busy}>
              打开文件
            </AppButton>
            <input
              ref={fileRef}
              type="file"
              accept="application/json,.json"
              hidden
              onChange={(e) => openFile(e.target.files?.[0])}
            />
          </div>

          <div className="project-recent">
            <div className="project-recent-head">data 中的工程</div>
            {recent.length === 0 ? (
              <div className="project-recent-empty">暂无</div>
            ) : (
              <ul className="project-recent-list">
                {recent.map((n) => (
                  <li key={n} className="project-recent-row">
                    <button
                      type="button"
                      className="project-recent-item"
                      disabled={busy}
                      onClick={() => void openSaved(n)}
                    >
                      <span>{n}</span>
                      <span className="project-recent-time">folder</span>
                    </button>
                    <button
                      type="button"
                      className="project-recent-delete"
                      disabled={busy}
                      onClick={() => void removeSaved(n)}
                    >
                      删除
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

import { useState } from 'react'
import type { Project } from '../../core/project'
import { saveProject } from './lib/projectStore'
import { Editor } from './ui/Editor'
import { ProjectWindow } from './ui/ProjectWindow'

export default function App() {
  const [session, setSession] = useState<{ project: Project; name: string } | null>(null)

  if (!session) {
    return (
      <ProjectWindow
        onOpen={(project, name) => setSession({ project, name })}
      />
    )
  }

  return (
    <Editor
      project={session.project}
      projectName={session.name}
      onChange={(project) => setSession({ ...session, project })}
      onBack={() => setSession(null)}
      onSave={(project) => {
        void saveProject(session.name, project)
      }}
    />
  )
}

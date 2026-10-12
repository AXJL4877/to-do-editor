import { spawn, type ChildProcess } from 'node:child_process'
import path from 'node:path'
import { app, BrowserWindow } from 'electron'

/** Windows 上 Electron35 易在 GPU/沙箱崩；CLI 已传 --no-sandbox --disable-gpu */
app.disableHardwareAcceleration()
app.commandLine.appendSwitch('disable-gpu-compositing')

/** 编译后位于 electron/dist/，上两级即仓库根 */
const ROOT = path.resolve(__dirname, '../..')
const isDev = !app.isPackaged

let backend: ChildProcess | undefined

function startBackend() {
  backend = spawn('npx', ['--yes', 'tsx', 'backend/src/server.ts'], {
    cwd: ROOT,
    shell: true,
    stdio: 'inherit',
    windowsHide: true,
  })
}

function createWindow() {
  const win = new BrowserWindow({
    width: 1440,
    height: 900,
    backgroundColor: '#0a0a0a',
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
    },
  })

  win.webContents.on('did-fail-load', (_e, code, desc, url) => {
    process.stderr.write(`load failed ${code} ${desc} ${url}\n`)
  })

  const devUrl = process.env.VITE_DEV_SERVER_URL || 'http://127.0.0.1:5173'
  if (isDev) {
    void win.loadURL(devUrl).catch((err) => {
      process.stderr.write(`dev url failed: ${String(err)}\n`)
      void win.loadURL(
        'data:text/html;charset=utf-8,' +
          encodeURIComponent(
            '<body style="background:#111;color:#ddd;font:14px sans-serif;padding:24px">' +
              '<h1>TO DO</h1><p>请先启动前端：<code>cd frontend && npm run dev</code></p>' +
              `<p>${devUrl}</p></body>`,
          ),
      )
    })
  } else {
    void win.loadFile(path.join(ROOT, 'frontend', 'dist', 'index.html'))
  }
}

app.whenReady().then(() => {
  startBackend()
  createWindow()
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('window-all-closed', () => {
  backend?.kill()
  if (process.platform !== 'darwin') app.quit()
})

app.on('before-quit', () => {
  backend?.kill()
})

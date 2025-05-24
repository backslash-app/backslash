import { app, BrowserWindow, globalShortcut, ipcMain, Menu, shell, Tray } from 'electron'
import { electronApp, is, optimizer } from '@electron-toolkit/utils'
import { join } from 'path'

import icon from '../../resources/icon.png?asset'

import {
  choosePluginsDir,
  getCommands,
  getHotkeys,
  getPluginActions,
  getPluginsDir,
  listInstalledApplications,
  openApplication,
  openExternal,
  runCommand,
  runPluginAction,
  setHotkey
} from './handlers'
import { setupAutoUpdater } from './autoUpdater'

let mainWindow: BrowserWindow
const gotTheLock = app.requestSingleInstanceLock()

const createWindow = async () => {
  mainWindow = new BrowserWindow({
    width: 600,
    height: 450,
    frame: false,
    show: false,
    ...(process.platform === 'linux' ? { icon } : {}),
    webPreferences: { preload: join(__dirname, '../preload/index.js'), sandbox: false }
  })

  mainWindow.on('ready-to-show', () => {
    mainWindow.show()
  })

  mainWindow.webContents.setWindowOpenHandler((details) => {
    shell.openExternal(details.url)
    return { action: 'deny' }
  })

  ipcMain.on('disable-global-shortcuts', () => globalShortcut.unregisterAll())
  ipcMain.on('enable-global-shortcuts', registerGlobalShortcut)

  mainWindow.on('blur', () => {
    if (!is.dev) mainWindow.hide()
  })

  mainWindow.on('close', (event) => {
    if (!app.isQuitting) {
      event.preventDefault()
      mainWindow.hide()
    }
    return false
  })

  if (is.dev && process.env['ELECTRON_RENDERER_URL']) {
    await mainWindow.loadURL(process.env['ELECTRON_RENDERER_URL'])
  } else {
    await mainWindow.loadFile(join(__dirname, '../renderer/index.html'))
  }
}

const registerGlobalShortcut = async () => {
  const hotkeys = await getHotkeys()
  const toggleAppHotkey = hotkeys['toggle-app'] || 'Ctrl+Space'

  globalShortcut.unregisterAll()
  globalShortcut.register(toggleAppHotkey, () => {
    if (mainWindow.isVisible()) mainWindow.hide()
    else mainWindow.show()
  })
}

const createTray = () => {
  if (process.platform !== 'linux') return

  global.tray = new Tray(icon)
  const contextMenu = Menu.buildFromTemplate([
    {
      label: 'Show app',
      click: () => mainWindow.show()
    },
    {
      label: 'Quit',
      click: () => {
        app.isQuitting = true
        app.quit()
      }
    }
  ])

  global.tray.setToolTip('Backslash')
  global.tray.setContextMenu(contextMenu)
}

if (!gotTheLock) {
  app.quit()
} else {
  app.on('second-instance', () => {
    if (mainWindow) {
      if (mainWindow.isMinimized()) mainWindow.restore()
      mainWindow.focus()
      mainWindow.show()
    }
  })

  app.whenReady().then(async () => {
    const { checkForUpdates } = setupAutoUpdater()

    electronApp.setAppUserModelId('com.electron')

    app.on('browser-window-created', (_, window) => {
      optimizer.watchWindowShortcuts(window)
    })

    await createWindow()
    await registerGlobalShortcut()
    createTray()

    ipcMain.handle('get-commands', () => {
      return getCommands()
    })

    ipcMain.handle('list-installed-applications', async () => {
      return listInstalledApplications()
    })

    ipcMain.handle('open-application', async (_, command) => {
      return openApplication(command)
    })

    ipcMain.handle('open-external', async (_, url) => {
      return openExternal(url)
    })

    ipcMain.handle('run-command', async (_, pluginName, commandName, params) => {
      return runCommand(pluginName, commandName, params)
    })

    ipcMain.handle('get-plugin-actions', async (_, pluginName, commandName) => {
      return getPluginActions(pluginName, commandName)
    })

    ipcMain.handle('run-plugin-action', async (_, pluginName, commandName, actionName, result) => {
      return runPluginAction(pluginName, commandName, actionName, result)
    })

    ipcMain.handle('choose-plugins-dir', async () => {
      return choosePluginsDir()
    })

    ipcMain.handle('get-plugins-dir', async () => {
      return getPluginsDir()
    })

    ipcMain.handle('set-hotkey', async (_, type, hotkey) => {
      return setHotkey(type, hotkey)
    })

    ipcMain.handle('get-hotkeys', async () => {
      return getHotkeys()
    })

    ipcMain.on('show-main-window', () => {
      if (mainWindow) mainWindow.show()
    })

    ipcMain.on('hide-main-window', () => {
      if (mainWindow) mainWindow.hide()
    })

    ipcMain.on('reload-app', () => {
      if (mainWindow) mainWindow.reload()
    })

    app.on('activate', async () => {
      if (BrowserWindow.getAllWindows().length === 0) {
        await createWindow()
      }
    })

    checkForUpdates()
  })
}

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit()
  }
})

import { app, clipboard, dialog, shell } from 'electron'
import os from 'os'
import storage from 'electron-json-storage'
import fs from 'fs'
import path from 'path'
import { exec } from 'child_process'
import axios from 'axios'
import cheerio from 'cheerio'
import yaml from 'js-yaml'

storage.setDataPath(os.tmpdir())

const DIRECTORIES = [
  '/usr/share/applications',
  '/usr/local/share/applications',
  '$HOME/.local/share/applications',
  '/var/lib/snapd/desktop/applications',
  '/var/lib/flatpak/exports/share/applications'
]

const EXCLUDED_PATTERNS = [
  /gnome/i,
  /org\.gnome/i,
  /Org/i,
  /kde/i,
  /xfce/i,
  /system/i,
  /Settings/i,
  /Preferences/i,
  /Configuration/i
]

const DEPS = {
  app,
  axios,
  cheerio,
  clipboard,
  exec,
  shell,
  path
}

/**
 * Gets all commands from plugins
 * @returns resolved with an array of command objects
 */
export const getCommands = async () => {
  const currentPluginsDir = await getPluginsDir()
  const plugins = fs.readdirSync(currentPluginsDir).filter((plugin) => plugin !== '.git')

  return plugins.flatMap((plugin) => {
    const manifestPath = path.join(currentPluginsDir, plugin, 'manifest.yml')
    const manifest = yaml.load(fs.readFileSync(manifestPath, 'utf8')) as ManifestT

    return manifest.commands.map((command) => ({
      ...command,
      plugin: {
        name: plugin,
        label: manifest.label,
        version: manifest.version,
        author: manifest.author
      }
    }))
  })
}

/**
 * Lists all installed applications
 * @returns promise resolved with an array of application objects, each
 * containing the application name and command
 */
export const listInstalledApplications = async () => {
  const formatAppName = (name: string): string => {
    return name
      .replace(/-/g, ' ')
      .split(' ')
      .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
      .join(' ')
  }

  const getAppCommand = (appPath: string): null | string => {
    const content = fs.readFileSync(appPath, 'utf-8')
    const execLine = content.split('\n').find((line) => line.startsWith('Exec='))

    if (execLine) {
      let command = execLine.replace('Exec=', '').trim()
      command = command.replace(/%\w+/g, '').trim()
      command = command.replace(/\s+/g, ' ')
      return command
    }

    return null
  }

  return new Promise((resolve, reject) => {
    const commands = DIRECTORIES.map((dir) => `ls ${dir}/*.desktop 2>/dev/null`).join(' || ')

    exec(commands, (error, stdout, stderr) => {
      if (error) return reject(error)
      if (stderr) return reject(new Error(stderr))

      const applications = stdout
        .split('\n')
        .filter((app) => app)
        .map((appPath) => {
          const appName = formatAppName(
            appPath
              .split('/')
              .pop()
              ?.replace(/\.desktop$/, '') || ''
          )

          const appCommand = getAppCommand(appPath)
          return { name: appName, command: appCommand, isImmediate: true }
        })
        .filter((app) => !EXCLUDED_PATTERNS.some((pattern) => pattern.test(app.name)))

      resolve(applications)
    })
  })
}

/**
 * Opens an application with the given command
 * @param command command to run to open the application
 * @returns promise resolved when the application is opened
 */
export const openApplication = async (command: string) => {
  return exec(command, (error) => {
    if (error) console.error(`Error opening application: ${error.message}`)
  })
}

/**
 * Opens a URL in the default external browser
 * @param url URL to open
 * @returns Promise resolved when the URL is opened
 */
export const openExternal = async (url: string) => {
  return shell.openExternal(url)
}

/**
 * Runs a command from a given plugin
 * @param pluginName name of the plugin
 * @param commandName name of the command
 * @param param parameter to pass to the command
 * @returns result of the command
 */
export const runCommand = async (pluginName: string, commandName: string, param: string) => {
  const currentPluginsDir = await getPluginsDir()
  const pluginPath = path.join(currentPluginsDir, pluginName, 'index.js')

  if (!fs.existsSync(pluginPath)) {
    throw new Error(`Plugin ${pluginName} not found`)
  }

  const plugin = require(pluginPath)
  if (typeof plugin.commands[commandName].run !== 'function') {
    throw new Error(`Command ${commandName} not found in plugin ${pluginName}`)
  }

  return await plugin.commands[commandName].run(param, DEPS)
}

/**
 * Gets the actions for a given plugin and command
 * @param pluginName name of the plugin
 * @param commandName name of the command
 * @returns list of actions for the given plugin and command
 */
export const getPluginActions = async (pluginName: string, commandName: string) => {
  const currentPluginsDir = await getPluginsDir()
  const pluginPath = path.join(currentPluginsDir, pluginName, 'index.js')

  if (!fs.existsSync(pluginPath)) {
    throw new Error(`Plugin ${pluginName} not found`)
  }

  const plugin = require(pluginPath)

  if (!plugin.commands[commandName] || !plugin.commands[commandName].actions) {
    throw new Error(`Actions not found for command ${commandName} in plugin ${pluginName}`)
  }

  return plugin.commands[commandName].actions.map((action) => ({
    name: action.name,
    description: action.description || '',
    shortcut: action.shortcut
  }))
}

/**
 * Runs a plugin action
 * @param pluginName name of the plugin
 * @param commandName name of the command
 * @param actionName name of the action
 * @param result result from the command
 * @returns result of the action
 */
export const runPluginAction = async (
  pluginName: string,
  commandName: string,
  actionName: string,
  result: ResultT
) => {
  const currentPluginsDir = await getPluginsDir()
  const pluginPath = path.join(currentPluginsDir, pluginName, 'index.js')

  if (!fs.existsSync(pluginPath)) {
    throw new Error(`Plugin ${pluginName} not found`)
  }

  const plugin = require(pluginPath)

  if (!plugin.commands[commandName] || !plugin.commands[commandName].actions) {
    throw new Error(`Actions not found for command ${commandName} in plugin ${pluginName}`)
  }

  const action = plugin.commands[commandName].actions.find((a) => a.name === actionName)?.action

  if (!action || typeof action !== 'function') {
    throw new Error(`Action ${actionName} not found or not a function`)
  }

  return await action(result, DEPS)
}

/**
 * Opens a file dialog for the user to choose a new plugins directory.
 * If the user chooses a directory, the path is stored in the user's
 * preferences and returned.
 * @returns the path to the new plugins directory, or null if the user
 * canceled.
 */
export const choosePluginsDir = async () => {
  const result = await dialog.showOpenDialog({ properties: ['openDirectory'] })

  if (!result.canceled && result.filePaths.length > 0) {
    const newPath = result.filePaths[0]
    await setPluginsDir(newPath)
    return newPath
  }

  return null
}

/**
 * Gets the directory where plugins are stored.
 * @returns a Promise that resolves with the path to the plugins directory.
 */
export const getPluginsDir = (): Promise<string> => {
  return new Promise((resolve) => {
    storage.get('pluginsDir', (error, data) => {
      if (error) throw error
      resolve(data as string)
    })
  })
}

/**
 * Sets the directory where plugins are stored.
 * @param newPath the path to the plugins directory to set.
 * @returns a Promise that resolves when the value has been set.
 */
const setPluginsDir = (newPath: string): Promise<void> => {
  return new Promise((resolve, reject) => {
    storage.set('pluginsDir', newPath, (error) => {
      if (error) reject(error)
      else resolve()
    })
  })
}

/**
 * Gets the current hotkeys.
 * @returns a Promise that resolves with the current hotkeys as an object.
 */
export const getHotkeys = (): Promise<{ [key: string]: string }> => {
  return new Promise((resolve) => {
    storage.get('hotkeys', (error, data) => {
      if (error) throw error
      resolve(data as { [key: string]: string })
    })
  })
}

/**
 * Sets a hotkey for the given type.
 * @param type the type of the hotkey (e.g. "toggle-app")
 * @param hotkey the hotkey to set (e.g. "Ctrl+Space")
 * @returns a Promise that resolves when the hotkey has been set.
 */
export const setHotkey = (type: string, hotkey: string): Promise<void> => {
  return new Promise((resolve, reject) => {
    storage.get('hotkeys', (error, data) => {
      if (error) reject(error)
      const hotkeys = (data as { [key: string]: string }) ?? {}
      hotkeys[type] = hotkey
      storage.set('hotkeys', hotkeys, (error) => {
        if (error) reject(error)
        else resolve()
      })
    })
  })
}

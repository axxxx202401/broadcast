#!/usr/bin/env node

/**
 * 为 Tauri 本地运行和打包加载指定环境。
 *
 * 本脚本只输出配置变量名和约束，不输出变量值。环境文件适合本地开发；CI 可以完全
 * 通过进程环境注入同名变量，且进程环境始终覆盖文件值。
 */

import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath, pathToFileURL } from 'node:url'

const COMMANDS = new Set(['dev', 'build', 'build-run'])
const PROFILES = new Set(['test', 'production'])
const REQUIRED_VARIABLES = [
  'IM_OPENCHAT_USER_URL',
  'IM_BIZ_URL',
  'IM_CHAT_HOST',
  'IM_CHAT_PORT',
  'IM_VERSION_SECRET_NAME',
  'IM_BODY_AES_KEY',
  'IM_HEADER_AES_KEY',
  'IM_APP_VER',
  'IM_PACKAGE_CODE',
  'IM_PLAT',
  'IM_LANGUAGE',
  'IM_SYS_MODEL',
  'VITE_GT4_CAPTCHA_ID',
]

/** 校验命令行选择，防止拼写错误悄悄选择其他环境。 */
export function validateInvocation(command, profile) {
  if (!COMMANDS.has(command)) {
    throw new Error('命令必须是 dev、build 或 build-run')
  }
  if (!PROFILES.has(profile)) {
    throw new Error('构建环境必须是 test 或 production')
  }
  return { command, profile }
}

/**
 * 解析简单 dotenv 文本。
 *
 * 支持空行、整行注释、可选 `export`、单引号和双引号；不执行变量展开，避免环境文件
 * 被当作脚本执行。
 */
export function parseEnvironmentFile(content) {
  const values = {}
  for (const [index, sourceLine] of content.split(/\r?\n/u).entries()) {
    let line = sourceLine.trim()
    if (!line || line.startsWith('#')) continue
    if (line.startsWith('export ')) line = line.slice('export '.length).trimStart()
    const separator = line.indexOf('=')
    if (separator <= 0) {
      throw new Error(`环境文件第 ${index + 1} 行格式无效`)
    }
    const name = line.slice(0, separator).trim()
    let value = line.slice(separator + 1).trim()
    if (!/^[A-Z_][A-Z0-9_]*$/u.test(name)) {
      throw new Error(`环境文件第 ${index + 1} 行变量名无效`)
    }
    const quoted = value.length >= 2
      && ((value.startsWith('"') && value.endsWith('"'))
        || (value.startsWith("'") && value.endsWith("'")))
    if (quoted) value = value.slice(1, -1)
    values[name] = value
  }
  return values
}

/** 合并环境文件与调用进程；调用者显式设置的值拥有最高优先级。 */
export function buildEnvironment(fileValues, processValues) {
  return { ...fileValues, ...processValues }
}

/**
 * 返回缺少关键 Node 包的安装目录。
 *
 * 只检查启动链路必需的包清单；任一清单缺失时对整个目录执行一次锁文件安装，既能覆盖
 * 首次克隆，也能修复 package.json 已增加依赖而 node_modules 尚未更新的情况。
 */
export function dependencyInstallTargets(appDirectory, pathExists = existsSync) {
  const uiDirectory = path.join(appDirectory, 'ui')
  const requirements = [
    {
      directory: appDirectory,
      manifests: [
        path.join(appDirectory, 'node_modules', '@tauri-apps', 'cli', 'package.json'),
      ],
    },
    {
      directory: uiDirectory,
      manifests: [
        path.join(uiDirectory, 'node_modules', 'vite', 'package.json'),
        path.join(uiDirectory, 'node_modules', 'vue', 'package.json'),
        path.join(uiDirectory, 'node_modules', '@tanstack', 'vue-virtual', 'package.json'),
      ],
    },
  ]
  return requirements
    .filter(({ manifests }) => manifests.some(manifest => !pathExists(manifest)))
    .map(({ directory }) => directory)
}

/** 返回当前工作区刚生成的可执行文件，避免误启动系统中安装的旧版本。 */
export function packagedExecutablePath(repositoryRoot, platform = process.platform) {
  if (platform === 'darwin') {
    return path.join(
      repositoryRoot,
      'target',
      'release',
      'bundle',
      'macos',
      'IM Monitor.app',
      'Contents',
      'MacOS',
      'im-app',
    )
  }
  return path.join(
    repositoryRoot,
    'target',
    'release',
    platform === 'win32' ? 'im-app.exe' : 'im-app',
  )
}

/** 在指定目录执行命令并继承终端输出，非零退出码直接终止启动流程。 */
function runCommand(executable, args, cwd, environment = process.env) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, {
      cwd,
      env: environment,
      stdio: 'inherit',
    })
    child.once('error', reject)
    child.once('exit', (code, signal) => {
      if (signal) {
        reject(new Error(`${executable} 被信号 ${signal} 终止`))
      } else if (code !== 0) {
        reject(new Error(`${executable} 退出码为 ${code ?? 'unknown'}`))
      } else {
        resolve()
      }
    })
  })
}

/** 首次运行或关键依赖缺失时，使用 package-lock 执行可重复安装。 */
export async function ensureNodeDependencies(appDirectory, options = {}) {
  const pathExists = options.pathExists ?? existsSync
  const platform = options.platform ?? process.platform
  const run = options.run ?? runCommand
  const npm = platform === 'win32' ? 'npm.cmd' : 'npm'
  for (const directory of dependencyInstallTargets(appDirectory, pathExists)) {
    const installMode = pathExists(path.join(directory, 'package-lock.json')) ? 'ci' : 'install'
    console.log(`检测到 Node 依赖未安装，正在执行 npm ${installMode}：${directory}`)
    await run(npm, [installMode], directory)
  }
}

/** 校验一组供 Rust 与 Vite 构建共同使用的配置。 */
export function validateEnvironment(environment) {
  const missing = REQUIRED_VARIABLES.filter((name) => !environment[name]?.trim())
  if (missing.length > 0) {
    throw new Error(`缺少必需构建变量：${missing.join(', ')}`)
  }

  validateHttpUrl(environment, 'IM_OPENCHAT_USER_URL')
  validateHttpUrl(environment, 'IM_BIZ_URL')
  validateHost(environment, 'IM_CHAT_HOST')
  validateInteger(environment, 'IM_CHAT_PORT', 1, 65_535)
  validateInteger(environment, 'IM_APP_VER', 0, 2_147_483_647)
  validateInteger(environment, 'IM_PACKAGE_CODE', 0, 2_147_483_647)
  validateInteger(environment, 'IM_PLAT', 0, 2_147_483_647)
  validateInteger(environment, 'IM_LANGUAGE', 0, 2_147_483_647)
  validateAes128Key(environment, 'IM_BODY_AES_KEY')
  validateAes128Key(environment, 'IM_HEADER_AES_KEY')
  validateText(environment, 'IM_VERSION_SECRET_NAME')
  validateText(environment, 'IM_SYS_MODEL')
  validateText(environment, 'VITE_GT4_CAPTCHA_ID')
}

function validateHttpUrl(environment, name) {
  try {
    const value = environment[name]
    const url = new URL(value)
    if (
      value.trim() !== value
      || !['http:', 'https:'].includes(url.protocol)
      || !url.hostname
      || url.username
      || url.password
      || url.search
      || url.hash
    ) throw new Error()
  } catch {
    throw new Error(`${name} 必须是绝对 HTTP(S) 基础 URL`)
  }
}

function validateHost(environment, name) {
  const value = environment[name]
  try {
    const url = new URL(`tcp://${value}`)
    if (
      !value
      || value.trim() !== value
      || !url.hostname
      || url.port
      || url.pathname
      || url.search
      || url.hash
      || url.username
      || url.password
    ) throw new Error()
  } catch {
    throw new Error(`${name} 必须是不含协议、端口、路径或空白的主机名/IP`)
  }
}

function validateInteger(environment, name, minimum, maximum) {
  const value = environment[name]
  if (!/^\d+$/u.test(value)) {
    throw new Error(`${name} 必须是整数`)
  }
  const parsed = Number(value)
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${name} 必须位于 ${minimum} 到 ${maximum} 之间`)
  }
}

function validateAes128Key(environment, name) {
  if (Buffer.byteLength(environment[name], 'utf8') !== 16) {
    throw new Error(`${name} 必须恰好为 16 字节 AES-128 key`)
  }
}

function validateText(environment, name) {
  const value = environment[name]
  if (!value || value.trim() !== value) {
    throw new Error(`${name} 不得为空或包含首尾空白`)
  }
}

/** 读取环境文件；文件不存在时允许 CI 完全通过进程环境提供配置。 */
async function readProfileFile(profile, repositoryRoot) {
  const environmentPath = path.join(repositoryRoot, 'config', `.env.${profile}`)
  try {
    return parseEnvironmentFile(await readFile(environmentPath, 'utf8'))
  } catch (error) {
    if (error?.code === 'ENOENT') return {}
    throw error
  }
}

/** 加载配置并启动项目本地安装的 Tauri CLI。 */
async function run() {
  const { command, profile } = validateInvocation(process.argv[2], process.argv[3])
  const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
  const repositoryRoot = path.resolve(scriptDirectory, '..')
  const appDirectory = path.join(repositoryRoot, 'im-app')
  const fileValues = await readProfileFile(profile, repositoryRoot)
  const environment = buildEnvironment(fileValues, process.env)
  validateEnvironment(environment)
  await ensureNodeDependencies(appDirectory)

  const tauriCommand = command === 'build-run' ? 'build' : command
  const tauriArgs = [tauriCommand, ...process.argv.slice(4)]
  console.log(`正在使用 ${profile} 环境执行 Tauri ${tauriCommand}`)

  if (process.platform === 'win32') {
    // Windows 上通过 cmd.exe 调用 .cmd 批处理文件，避免 spawn EINVAL
    const tauriCmd = path.join(appDirectory, 'node_modules', '.bin', 'tauri.cmd')
    await runCommand('cmd.exe', ['/c', tauriCmd, ...tauriArgs], appDirectory, environment)
  } else {
    const executable = path.join(appDirectory, 'node_modules', '.bin', 'tauri')
    await runCommand(executable, tauriArgs, appDirectory, environment)
  }

  if (command === 'build-run') {
    const application = packagedExecutablePath(repositoryRoot)
    if (!existsSync(application)) {
      throw new Error(`构建成功但未找到当前工作区应用：${application}`)
    }
    console.log(`正在启动当前工作区应用：${application}`)
    await runCommand(application, [], appDirectory, environment)
  }
}

const invokedPath = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : ''
if (import.meta.url === invokedPath) {
  run().catch((error) => {
    console.error(error instanceof Error ? error.message : '环境构建脚本执行失败')
    process.exitCode = 1
  })
}

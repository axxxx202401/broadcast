import assert from 'node:assert/strict'
import path from 'node:path'
import test from 'node:test'

import {
  buildEnvironment,
  dependencyInstallTargets,
  ensureNodeDependencies,
  packagedExecutablePath,
  parseEnvironmentFile,
  validateEnvironment,
  validateInvocation,
} from './tauri-env.mjs'

const completeEnvironment = {
  IM_OPENCHAT_USER_URL: 'https://user.example.com',
  IM_BIZ_URL: 'https://biz.example.com',
  IM_CHAT_HOST: 'chat.example.com',
  IM_CHAT_PORT: '9500',
  IM_VERSION_SECRET_NAME: 'version-secret',
  IM_BODY_AES_KEY: '1234567890abcdef',
  IM_HEADER_AES_KEY: 'abcdef1234567890',
  IM_APP_VER: '680',
  IM_PACKAGE_CODE: '9803',
  IM_PLAT: '0',
  IM_LANGUAGE: '2',
  IM_SYS_MODEL: 'PC-TOOLS',
  VITE_GT4_CAPTCHA_ID: 'captcha-id',
}

test('只接受已声明的 Tauri 命令和构建环境', () => {
  assert.deepEqual(validateInvocation('dev', 'test'), { command: 'dev', profile: 'test' })
  assert.deepEqual(validateInvocation('build', 'production'), {
    command: 'build',
    profile: 'production',
  })
  assert.deepEqual(validateInvocation('build-run', 'test'), {
    command: 'build-run',
    profile: 'test',
  })
  assert.throws(() => validateInvocation('serve', 'test'), /dev、build 或 build-run/)
  assert.throws(() => validateInvocation('build', 'staging'), /test 或 production/)
})

test('一键构建运行始终指向当前工作区产物而非系统已安装应用', () => {
  const root = path.resolve('/repo')
  assert.equal(
    packagedExecutablePath(root, 'darwin'),
    path.join(root, 'target', 'release', 'bundle', 'macos', 'IM Monitor.app', 'Contents', 'MacOS', 'im-app'),
  )
  assert.equal(
    packagedExecutablePath(root, 'win32'),
    path.join(root, 'target', 'release', 'im-app.exe'),
  )
  assert.equal(
    packagedExecutablePath(root, 'linux'),
    path.join(root, 'target', 'release', 'im-app'),
  )
})

test('解析注释、export、引号和包含等号的值', () => {
  assert.deepEqual(
    parseEnvironmentFile(`
      # 构建配置
      export IM_CHAT_HOST=chat.example.com
      IM_VERSION_SECRET_NAME="name=value"
      IM_SYS_MODEL='PC-TOOLS'
    `),
    {
      IM_CHAT_HOST: 'chat.example.com',
      IM_VERSION_SECRET_NAME: 'name=value',
      IM_SYS_MODEL: 'PC-TOOLS',
    },
  )
  assert.throws(() => parseEnvironmentFile('BAD LINE'), /第 1 行/)
})

test('调用进程变量覆盖环境文件，同时保留 PATH 等系统变量', () => {
  const environment = buildEnvironment(
    { IM_CHAT_HOST: 'file-host', PATH: 'file-path' },
    { IM_CHAT_HOST: 'process-host', PATH: 'process-path', HOME: '/tmp/home' },
  )

  assert.equal(environment.IM_CHAT_HOST, 'process-host')
  assert.equal(environment.PATH, 'process-path')
  assert.equal(environment.HOME, '/tmp/home')
})

test('完整配置通过校验', () => {
  assert.doesNotThrow(() => validateEnvironment(completeEnvironment))
})

test('缺失和非法配置只报告变量名，不泄漏变量值', () => {
  const missing = { ...completeEnvironment }
  delete missing.IM_BODY_AES_KEY
  assert.throws(() => validateEnvironment(missing), /IM_BODY_AES_KEY/)

  const invalid = { ...completeEnvironment, IM_HEADER_AES_KEY: 'sensitive-bad-key' }
  assert.throws(
    () => validateEnvironment(invalid),
    (error) => {
      assert.match(error.message, /IM_HEADER_AES_KEY/)
      assert.doesNotMatch(error.message, /sensitive-bad-key/)
      return true
    },
  )

  for (const [name, value] of [
    ['IM_OPENCHAT_USER_URL', 'ftp://user.example.com'],
    ['IM_BIZ_URL', 'http://biz.example.com:bad-port'],
    ['IM_CHAT_PORT', '0'],
    ['IM_APP_VER', '6.80'],
    ['IM_CHAT_HOST', 'https://chat.example.com/path'],
    ['IM_CHAT_HOST', 'chat.example.com/'],
    ['IM_VERSION_SECRET_NAME', ' secret '],
    ['IM_SYS_MODEL', ' PC-TOOLS'],
    ['VITE_GT4_CAPTCHA_ID', ''],
  ]) {
    assert.throws(
      () => validateEnvironment({ ...completeEnvironment, [name]: value }),
      new RegExp(name),
    )
  }
})

test('只为缺失关键包的应用目录生成依赖安装任务', () => {
  const appDirectory = path.resolve('/repo/im-app')
  const existing = new Set([
    path.join(appDirectory, 'node_modules', '@tauri-apps', 'cli', 'package.json'),
  ])
  const targets = dependencyInstallTargets(
    appDirectory,
    candidate => existing.has(candidate),
  )

  assert.deepEqual(targets, [path.join(appDirectory, 'ui')])
})

test('首次运行时为 Tauri 与 UI 两个目录生成安装任务', () => {
  const appDirectory = path.resolve('/repo/im-app')
  assert.deepEqual(
    dependencyInstallTargets(appDirectory, () => false),
    [appDirectory, path.join(appDirectory, 'ui')],
  )
})

test('依赖缺失时按目录自动执行锁文件安装', async () => {
  const appDirectory = path.resolve('/repo/im-app')
  const calls = []
  const existing = new Set([
    path.join(appDirectory, 'package-lock.json'),
    path.join(appDirectory, 'ui', 'package-lock.json'),
  ])

  await ensureNodeDependencies(appDirectory, {
    pathExists: candidate => existing.has(candidate),
    platform: 'linux',
    run: async (executable, args, cwd) => calls.push({ executable, args, cwd }),
  })

  assert.deepEqual(calls, [
    { executable: 'npm', args: ['ci'], cwd: appDirectory },
    { executable: 'npm', args: ['ci'], cwd: path.join(appDirectory, 'ui') },
  ])
})

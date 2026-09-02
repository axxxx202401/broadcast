import { describe, expect, it } from 'vitest'

import appPackage from '../../package.json'
import uiPackage from '../package.json'

describe('Tauri npm 工作目录兼容性', () => {
  it('保留 UI 本地脚本并由 im-app 代理同名命令', () => {
    expect(uiPackage.scripts.dev).toBe('vite')
    expect(uiPackage.scripts.build).toBe('vue-tsc --noEmit && vite build')
    expect(appPackage).toMatchObject({
      private: true,
      scripts: {
        dev: 'npm --prefix ui run dev',
        build: 'npm --prefix ui run build',
      },
    })
  })
})

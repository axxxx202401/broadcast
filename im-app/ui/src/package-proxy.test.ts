import { describe, expect, it } from 'vitest'

import appPackage from '../../package.json'
import uiPackage from '../package.json'

describe('Tauri npm cwd compatibility', () => {
  it('keeps UI scripts local and proxies the same commands from im-app', () => {
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

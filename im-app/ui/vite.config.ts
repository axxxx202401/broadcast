import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

import tauriConfig from '../tauri.conf.json'

const devCsp = tauriConfig.app.security.devCsp as Record<string, string>
export const DEV_CSP_HEADER = Object.entries(devCsp)
  .map(([directive, value]) => `${directive} ${value}`)
  .join('; ')

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    headers: {
      'Content-Security-Policy': DEV_CSP_HEADER,
    },
  },
  test: {
    include: ['src/**/*.test.ts'],
  },
})

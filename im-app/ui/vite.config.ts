import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

import tauriConfig from '../tauri.conf.json'

/**
 * 直接从 Tauri `app.security.devCsp` 序列化开发服务器响应头，避免两处开发策略漂移。
 * 该对齐只保证配置来源一致，并不意味着 CSP 能覆盖所有安全风险。
 */
const devCsp = tauriConfig.app.security.devCsp as Record<string, string>
export const DEV_CSP_HEADER = Object.entries(devCsp)
  .map(([directive, value]) => `${directive} ${value}`)
  .join('; ')

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    // 与 Tauri `build.devUrl` 对齐，只监听回环地址；端口占用时直接失败，避免静默切换。
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    headers: {
      'Content-Security-Policy': DEV_CSP_HEADER,
    },
  },
  test: {
    // 仅收集源码树内显式命名的 Vitest 测试文件。
    include: ['src/**/*.test.ts'],
  },
})

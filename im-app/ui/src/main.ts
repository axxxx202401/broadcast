import { createApp } from 'vue'

import App from './App.vue'
import './styles/base.css'
import './styles/console.css'

// 应用入口：加载全局样式，并将根组件挂载到宿主页面。
const app = createApp(App)

/**
 * Vue 全局错误处理器：捕获渲染或生命周期错误并记录，不中断应用。
 * 仅记录错误类型与提示信息，避免泄露用户数据。
 */
app.config.errorHandler = (err, instance, info) => {
  console.error('[Vue Error]', err instanceof Error ? err.message : String(err), info)
}

app.mount('#app')

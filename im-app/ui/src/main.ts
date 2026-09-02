import { createApp } from 'vue'

import App from './App.vue'
import './styles/base.css'
import './styles/console.css'

// 应用入口：加载全局样式，并将根组件挂载到宿主页面。
createApp(App).mount('#app')

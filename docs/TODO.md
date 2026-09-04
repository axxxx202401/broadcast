# 开发任务清单

## 1. 消息匹配功能

### 需求
- 消息内容含有"开奖"并且含有对应的"期号"则为可显示消息
- 增加 `matched` 字段持久化
- 提供开关控制是否只显示匹配的消息，开关跟着用户走

### 已完成
- [x] 数据库 schema 增加 `matched` 字段
- [x] 消息匹配逻辑实现
- [x] matched 字段持久化
- [x] UI 开关（显示/隐藏匹配消息）
- [x] 修复测试：所有 `get_by_group()` 和 `get_recent()` 调用补充 `matched_only` 参数
- [x] 修复测试：`MessageRow` 和 `MessageDto` 初始化补充 `matched` 字段

---

## 2. 开奖信息面板

### 需求
- API: `https://go124.com/api/hash/get28HistoryList/10091`
- 响应结构：`{"success":true,"result":{"list":[{"preDrawIssue":xxx,"preDrawTime":"..."}]}}`
- 显示本期和上期期号（最新两条），突出期号，弱化时间
- 提供 API URL 配置，保存后生效
- 每 30 秒轮询，收到含"开奖"消息时额外触发

### 已完成
- [x] `im-http/src/lottery.rs` - API 调用和 JSON 解析
- [x] `im-app/src/commands/lottery.rs` - Tauri 命令
- [x] `im-store/src/lottery_config.rs` - 配置持久化
- [x] `LotteryPanel.vue` - 界面组件
- [x] `useLottery.ts` - 业务逻辑 composable
- [x] 修复参数名：camelCase 正确传递
- [x] 移除当前期号编辑（用户说不需要）
- [x] 添加 serde rename 属性映射 camelCase JSON 字段
- [x] 添加调试日志追踪 API 响应结构

### 已完成
- [x] 修复数据库坏 URL（`100912` → `10091`）
- [x] 改进错误提示：`success=false` 时返回服务端 message，`result=null` 时提示检查期号
- [x] LotteryPanel 嵌入消息区顶部（标题栏下方，全宽横贯，不挤占消息内容区）
- [x] 群列表增加「收起」按钮（◀），点击后侧栏整体收起

### 已完成
- [x] 运行时 API 解析正常（测试 + 修复后验证）
- [x] 保存配置后 URL 正确持久化并显示
- [x] 构建产物为最新代码

---

## 3. 群列表优化

### 需求
- 群列表增加收缩/展开功能
- 分两类显示：监听中 / 未监听

### 已完成
- [x] `GroupSidebar.vue` 重构为两 sections
- [x] 监听中的群显示在上 section
- [x] 未监听的群显示在下 section
- [x] 每个 section 可独立收缩/展开
- [x] 修复测试：`GroupSidebar.test.ts` 适配新结构
- [x] 群列表增加整体收起按钮（◀），点击后触发 `layout.closeSidebar`

---

## 4. 测试状态

### 测试结果
- [x] `im-store/src/tests.rs` - 25 tests pass
- [x] `im-http/src/lottery.rs` - lottery test pass
- [x] `im-app/ui/` - 213 tests pass

---

## 已解决的问题

### API 解析运行时失败（已修复）
- **根因**：用户保存的 URL 为 `.../100912`（多按了 `2`），服务端返回 `{"success":false,"result":null}` ，原代码未处理此结构
- **修复**：
  1. 清理数据库坏 URL（`100912` → `10091`）
  2. 新增 `success=false` 检查，返回服务端 `message` 内容（如"彩种不存在."）
  3. LotteryPanel 移到群列表与消息区分界处，不再占用顶部空间

---

## 启动命令

```bash
npm run dev:test
```

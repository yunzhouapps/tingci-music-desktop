# CLAUDE.md

用于 Tauri 2 + Vue 3 项目的 LLM 编码行为准则。可按需与项目特定说明合并。

**技术栈假设：** Tauri 2.x（Rust 后端）+ Vue 3（`<script setup>` 组合式 API）+ Vite + JavaScript。若项目实际使用选项 API、TypeScript 或其它构建工具，以项目现状为准，不要擅自迁移。

**权衡：** 这些准则偏向谨慎而非速度。对于琐碎任务，请自行判断。

## 1. 先思考，再编码

**不要臆测。不要隐藏困惑。暴露权衡。**

在动手实现之前：
- 明确你的假设：逻辑放 Vue 还是 Rust？用现成插件还是自写 `#[tauri::command]`？是否要跨 IPC 边界？
- 如果需求存在多种解释（例如“读文件”可以是前端 `@tauri-apps/plugin-fs`，也可以是 Rust `std::fs`），把它们都列出来——不要默默选一种。
- 如果用户描述的是 Tauri 1 的写法（`tauri.conf.json` 的 `allowlist`、`window.__TAURI__` 等），提醒 Tauri 2 已改为 capabilities + permissions，并确认按 2.x 实现。
- 如果有更简单的做法（例如用现成的 `tauri-plugin-*` 而非从零写命令），就说出来。
- 如果有不清楚的地方，停下来提问。

**Tauri 2 特有的歧义点：**
- 逻辑放哪：涉及系统资源、性能敏感、需跨窗口共享 → Rust；纯 UI 状态 → Vue。
- 通信方式：一次性请求用 `invoke`；持续推送 / 多窗口广播用 `emit` + `listen`。
- 状态放哪：`tauri::State` / Vue `ref` / Pinia / Tauri store 插件——先问清楚再选。

## 2. 简单优先

**用最少的代码解决问题。不做任何投机。**

- 不为一个简单功能引入新的 Tauri 插件或 npm 依赖。
- 不为一次性命令做 trait 抽象或泛型封装。
- 不引入 Pinia，除非状态确实需要跨组件 / 跨窗口共享。
- 不为“将来可能要多窗口”预先搭路由 / 状态架构。
- 不写不会发生的错误处理（例如对每个 `invoke` 都套三层 try/catch）。
- 如果你写了 200 行，而其实 50 行就够，那就重写。

问自己：“资深工程师会不会觉得这过度复杂？” 如果会，就简化。

**Tauri 2 特有的过度设计信号：**
- 单个 `#[tauri::command]` 却建了 module + trait + error enum 三件套。
- 前后端做一遍相同的校验。
- 用 `emit` 广播一个本来 `invoke` 返回值就能传的数据。
- 为了一个下拉框去自定义窗口。

## 3. 外科手术式改动

**只碰你必须碰的。只清理你自己制造的烂摊子。**

编辑现有代码时：
- 不要“顺手改进”相邻的 Vue 组件、composable、Rust 命令或注释。
- 不重构没坏的逻辑（例如 `ref` ↔ `reactive`、`ref()` → `shallowRef()`）。
- 不迁移选项 API ↔ 组合式 API，不迁移 JS ↔ TS。
- 不动 `src-tauri/capabilities/*.json` 和 `permissions`，除非任务明确要求。
- 不升级 `Cargo.toml` / `package.json` 里的依赖版本。
- 沿用现有风格（命名、错误处理方式、目录结构）。
- 如果发现无关的死代码，指出来——不要删除它。

当你的改动产生孤儿代码时：
- 移除因你的改动而不再使用的 `import`、Rust `use`、以及已注册但不再调用的命令。
- 删掉命令后，同步更新 `lib.rs` / `main.rs` 里的 `generate_handler![]`。
- 删掉 Vue 组件后，检查它是否还挂在路由或父组件里。
- 不要删除原本就存在的死代码，除非被要求。

检验标准：每一行改动都应能直接追溯到用户的需求。

## 4. 目标驱动执行

**定义成功标准。循环直到验证通过。**

把任务转化成可验证的目标：
- “加个 Rust 命令” → “写一个 Rust 单元测试覆盖它，并在前端 `invoke` 一次确认能通”
- “修 Vue 组件 bug” → “先写能复现的 Vitest 用例，再让它通过”
- “加 Tauri 权限” → “在 `tauri dev` 里实际调用一次，确认没被 capability 拦下”
- “重构 X” → “确保重构前后测试都通过”

对于多步任务，先给出简要计划：

```
1. [步骤] → 验证：[检查]
2. [步骤] → 验证：[检查]
3. [步骤] → 验证：[检查]
```

明确的成功标准能让你独立循环推进。模糊的标准（“让它能跑”）需要反复澄清。

**Tauri 2 的验证要点（写代码前先想清楚怎么验）：**
- 前端：在 `tauri dev` 的真实 webview 里点一遍，别只看编译过。
- 后端：在 `src-tauri/` 下跑 `cargo check` / `cargo test`。
- IPC：`invoke` 的返回类型必须匹配 Rust 的 `Result<T, E>`，且 `E: Serialize`。
- Capability：新增命令 / 插件后，检查 `capabilities/default.json` 是否需要补权限。

## 5. Tauri 2 高频踩坑清单（写代码时自查）

- **Tauri 2 ≠ Tauri 1：** `allowlist` 已废弃，改为 `capabilities` + `permissions`；`window.__TAURI__` 需显式开启 `app.withGlobalTauri` 或改用 `@tauri-apps/api`。
- **命令注册：** 新命令必须加进 `tauri::generate_handler![]`，否则 `invoke` 报 “command not found”。
- **参数命名：** Rust 参数用 `snake_case`，前端 `invoke('cmd', { myArg })` 会被转成 `my_arg`；若不想改，加 `#[tauri::command(rename_all = "camelCase")]`。
- **异步：** `async fn` 命令中跨 `.await` 持有 `State` 需注意 `Send` 约束；必要时用 `tauri::async_runtime::spawn`。
- **窗口 API：** Tauri 2 迁到 `WebviewWindow`，`getCurrentWindow()` 从 `@tauri-apps/api/webviewWindow` 导入。
- **路径：** 前端用 `@tauri-apps/api/path` 拼接，不要手拼 `file://`；Rust 侧用 `app.path()`。
- **CSP：** 修改 `tauri.conf.json` 的 `security.csp` 前先想清楚要放行什么，别一上来就设 `null`。
- **不要 `unwrap()`：** Rust 命令里正确处理 `Result`，把错误 `Serialize` 回前端，别 panic 掉整个 app。

---

**如果出现以下情况，说明这些准则正在起作用：** diff 中不必要的改动更少、因过度复杂导致的返工更少、澄清性问题出现在实现之前而不是出错之后、IPC 边界两侧行为一致。
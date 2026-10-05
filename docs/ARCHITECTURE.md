# Relay 技术架构

更新：2026-10-05。当前采用常驻主 Agent、独立任务 thread 和 Claude-Mem 记忆 Provider。Project 与 Jev 不再参与消息流。详细一致性和存储设计见 [Memory Provider](MEMORY-PROVIDERS.md)。

## 运行模型

```text
主窗口 / 全局快捷入口 / 语音
                 │ 明确入口
      Relay 主对话（logical thread 0）
                 │ ACP + 本地 stdio MCP
           常驻主 Agent（Codex）
                 │ create_task / continue_task / inspect_task
      SQLite 持久队列 + ResidentWorker
             ┌───┴───┐
          任务 thread  任务 thread
             │ 独立 ACP session
             └──结果 outbox──→ 主对话审阅

用户消息 / ACP 工具结果 / 轮次终态 / 显式历史导入
                 │ MemoryProvider
        本地持久投递队列 delivery.sqlite3
                 │ Worker HTTP API
           独立 Claude-Mem Worker
                 │ 提炼模型 / 总结 / 索引
          observations / summaries / Chroma
                 │ 渐进检索与上下文 API
           Relay MCP / 记忆页面 / prompt
```

用户显式打开任务后，输入直接进入该任务。同一句 prompt 不经过主题分类器决定“当前还是新 thread”。主 Agent 可以在一次主对话中协调多项工作；独立执行是工具调用结果。

## 包边界

| 包 | 当前职责 |
| --- | --- |
| `relay` | 依赖装配、原生窗口、退出生命周期、headless MCP 入口 |
| `relay-core` | thread、任务活动、记忆、消息、文件、语音等纯领域类型与服务端口 |
| `relay-ui` | GPUI Kit 工作台、任务活动、记忆管理、会话、文件、快捷入口、语音浮层 |
| `relay-runtime` | 任务数据库、记忆 Provider/投递队列、Agent 编排、历史归档、个人文件、偏好、ASR/TTS |
| `relay-acp` | ACP SDK、stdio 进程、认证、配置、流式事件、权限、取消、新建与恢复时的 MCP 配置 |
| `relay-platform` | macOS 快捷键、选区、窗口、采音、KWS/VAD 和播放 |
| `xtask` | 固定依赖、包边界、质量检查、资源校验、签名和打包 |

UI 不依赖 ACP SDK、SQLite 或 runtime；入口注入领域服务。SQLite 留在 runtime，使用固定 `rusqlite` 及 bundled SQLite；提炼与向量索引由 Claude-Mem 持有。其他依赖和平台约束见 [包治理](PACKAGES.md)。

## 会话和上下文

逻辑 thread 是 durable identity，ACP session 是可替换执行上下文。主对话为 0，任务使用单调 ID，旧 Observer 保留 ID 禁止执行。`runtime_sessions` 保存当前绑定，`owned_sessions` 保存所有 Relay 创建的原生 session，避免历史导入重复消费 Relay 自己的会话。

连接 generation 阻止旧进程事件污染新会话；模型选项从 ACP `configOptions` 获取，只有收到确认后才更新偏好。原生恢复成功沿用上下文检查点；失败时创建新执行上下文，注入有限可见历史和当前资料，逻辑 thread 不改变。

thread 的目标、显式选用资料采用快照/增量交付。常驻角色说明、有限任务状态和相关记忆索引作为运行上下文，在 prompt 后台准备阶段注入。完整记忆和历史通过工具按需展开。原有 `MemoryItem`/JSON 存储类型仅作为旧数据与测试兼容层；应用装配使用 `ResidentStore` 与独立 `MemoryProvider`，观察与总结来自 Worker。

## 调度与恢复

任务工具先提交持久队列，worker 再连接或复用任务 ACP session。连接与推理不会阻塞 MCP 创建调用。任务最多四路派发，结果按 assistant message ID 关联；保存结果与通知主对话共用事务。直接进入任务的追问也登记执行记录。

主对话仅在 Ready 且没有待确认配置时接收后台结果。通知标明为后台 task event，不伪装成新用户指令；可能触发主 Agent 的整合回复。任务停止、失败、等待授权和完成由可见协议状态表达，成功执行先待审阅。

重启不重放结果未知的执行请求；connecting 请求尚未发送，可以重新排队。记忆投递队列独立恢复。离线事件保留；POST 结果未知时需先检查 Worker 再显式处理，不盲目重放。已进入队列的事件可跨重启投递。对话保存与记忆日志间仍有崩溃窗口，历史扫描补录需开启历史导入；不承诺跨两个存储的原子提交。

## 记忆与历史

采集先写 Relay 投递日志，Worker 接收后异步提炼观察与总结。接收不等于提炼成功，也不等于用户确认。渐进检索采用 `memory_search → memory_timeline → memory_get`，作用域由 Relay namespace 和 thread 来源限制，具体检索由上游完成。

旧 Codex 历史继续只读归档，其他 Client 解析器尚未实现。将这些历史交给 Worker 需要显式开启导入；不接管外部正在运行的 session。Native 的确认、修订、来源级遗忘、主题页与提炼预算已移除。

## 文件、桌面与语音

个人文件目录独立于 thread，任务成果统一展示在 Files。显式并行写同一文件仍可能冲突；代码工作应由 Agent 选择独立 worktree/工作目录，Relay 没有自动建立代码沙箱。

快捷键采集选区和可取得的 URL，Moli 在后台补充网页上下文；发送冻结已取得材料，迟到内容不修改在途输入。默认目标为主对话，小窗可显式选任务，展开保持同一 thread。

语音为本地 KWS/VAD → 千问 ASR → 主对话 → 千问 TTS。处理/播放期间暂停收音，回复与取消绑定本轮 assistant ID；单句上限 60 秒，音频不落盘。独立语音密钥与本地主 Agent 配置继续沿用。截图、持续视觉和更多 Harness 保留为后续能力。

## 记忆界面与归档边界

`relay-core::memory::MemoryService` 提供观察分页、详情、能力信息和操作。GPUI 通过后台 executor 调用，显示期间每三秒刷新，使用请求序号丢弃旧筛选结果。页面展示投递健康、观察内容、概念标签、单条删除与失败重试。来源/主题页及候选确认修订界面已经移除。

`ClientSessionStore` 继续负责原生 JSONL 归档、增量同步和读取；它与记忆提炼分离。原始数据、旧记忆与升级规则见 [Memory Provider](MEMORY-PROVIDERS.md)。

# 本地 Client 会话归档与记忆导入

Relay 使用用户本地安装的 Harness。后台归档服务的同步对象是本地 Client 的原生会话，包含用户在 CLI、桌面 Client 等入口发起的对话。首版接入 Codex；Claude Code、OpenCode 的解析器后续按同一服务边界扩展。

## 记忆入口

左侧入口为 **记忆**。原来的“本地会话”独立 Tab、手动归属、按任务筛选及工作目录自动匹配已移除。原生会话 ID 只标识原始会话，不决定任务身份或输入目标。

记忆页面现在通过 `MemoryService` 展示 Claude-Mem 观察、详情、概念和投递状态，支持搜索、单条观察删除与明确失败的投递重试。旧 Native 的来源/主题页、候选确认、版本修订和提炼预算控制已经移除，页面不再提供原始会话查看入口。只读归档服务与文件继续保留。

归档成功只表示可见对话已存到 Relay 本地，不表示已经进入 Worker。`Relay --memory-provider-import-history on` 显式开启后，`ResidentWorker` 扫描这些归档，通过同一 Provider 生命周期队列提交用户/助手内容供 Claude-Mem 提炼。默认不导入旧聊天。Relay 自己创建的 ACP session 和旧 Observer 工作目录均被排除，避免重复/递归导入。

已接受的历史事件不模拟来源版本或 tombstone；归档修改不会自动撤回已经产生的观察。删除 observation 也不会删除原始 prompt、工具事件或 session summary。详见 [Memory Provider](MEMORY-PROVIDERS.md)。

## 快照与增量

启动时同步一次，每次同步结束后等待 **30 分钟**；记忆页的旧“同步来源”按钮已移除。该间隔只控制本地会话归档；任务资料与记忆仍随下一条 ACP 消息交付。

Codex 来源为 `$CODEX_HOME/sessions` 和 `$CODEX_HOME/archived_sessions`，未设置时使用 `~/.codex`。读取 JSONL 的 `session_meta` 与 `response_item` 中的可见用户/助手文字，忽略开发者指令、推理、工具输出、重复事件和已知环境注入项。不连接或接管用户已经运行的对话。

每个会话保存自己的归档、最后完整记录的字节位置及已提交前缀的 SHA-256。文件长度、修改时间和文件身份均未变化时跳过；变化时校验已提交前缀，追加只解析新增完整行，截短或改写则重新建立当前快照。校验变更文件的旧前缀仍需读取旧字节，不重复解析、生成摘要或调用模型。未写完的尾行不推进游标。

原生会话 ID 用于去重，移动到 archived_sessions 后保持同一会话身份。原始文件只读。损坏的完整记录不推进快照；读取失败时保留已有归档。源文件消失后保留已导入的记录并显示来源不可用。未知版本或损坏的 Relay 归档、目录不会被覆盖。

单条 JSONL 记录读取上限为 64 MB，超出时保留已有快照并提示同步失败；压缩上下文等非对话记录不会写入归档。Relay 归档被删除而来源仍在时，下次同步可重新建立快照。

## 存储与边界

```text
Relay/
  client-sessions/index.json       # 会话目录、来源文件状态、同步时间
  client-sessions/archives/<hash>.json  # 完整可见文字及同步游标
  relay.sqlite3                   # 任务、会话绑定和消息编号；旧记忆表保留但不再使用
  memory-providers/<hash>/delivery.sqlite3  # Worker 投递日志；不是记忆库
```

归档与目录采用 0600 文件权限及原子替换。目录只保存元数据；打开会话时在后台加载文字。导入本身不执行模型请求、不修改原生记录；后续提炼遵循 Claude-Mem 的配置。Relay 不再建立原文 FTS 或自己的 Embedding 索引。

外部 session 仍是只读导入，尚不支持接管原生运行中会话或原生续聊。Relay 内部独立任务已经实现，使用新的 ACP session；内部 session 和已退役 Observer 输出不重复当作外部记忆来源。

## 历史归档验证记录（不代表新版记忆验收）

2026-09-30 在隔离的 Relay 数据目录导入 2,131 条真实 Codex 会话，未修改来源文件；随后重复同步更新数为 0，失败数为 0。测试覆盖外部追加、重启游标、部分尾行、文件改写、大型压缩上下文、归档重建和损坏文件保护；当时的归属与旧记忆界面现已移除。

可运行 `cargo run -p relay-runtime --example client_sessions_probe --locked -- --data-dir <隔离目录>` 复测；该命令只打印统计，不发起 ACP 连接或模型请求。

2026-10-04 当时的 Native 引擎回归覆盖完整长文本、改写与移除片段失效、来源版本、遗忘 tombstone、多 session 主题集合，以及任务与 ACP session 身份分离。这套记忆引擎现已移除，不能作为当前 Claude-Mem 验收结果。

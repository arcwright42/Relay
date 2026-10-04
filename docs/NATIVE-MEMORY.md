# 常驻 Agent 与原生记忆

实施日期：2026-10-04。此方案取代 Project / Jev 层级以及将全部薄记忆放进每条 prompt 的方案。

## 身份与消息目标

Relay 的持久身份为 `relay-resident`，主对话为逻辑 `ThreadId(0)`。普通输入、全局快捷入口和语音都进入主对话；从任务详情输入或在快捷面板明确选择任务时，消息进入该任务。话题改变不会创建或切换 thread。

主 Agent 在当前对话中判断是否需要独立执行工作，通过 `create_task` 委派，通过 `continue_task` 继续已明确识别的任务。它先查 `list_tasks` / `inspect_task`；指代不清时在主对话澄清。相似主题只用于查找材料，不决定消息投递。任务的 `ThreadId`、Relay 可见消息 ID、ACP session ID 是不同标识。

“常驻”是应用进程中的调度器与持久状态；模型按消息和后台任务调用，不需要一直生成 token。关闭窗口后继续运行，退出应用后停止。逻辑主对话保持身份，底层 session 可以恢复或重建；ACP 原生上下文压缩仍由 Harness 管理。

## 参考与实现边界

参考 [claude-mem](https://github.com/thedotmack/claude-mem/tree/bfc50259f64a4e0c9e81df849095683608e87ac8) 的事件采集、后台 observation、session 摘要、来源追溯和渐进检索。Relay 使用自己的 Rust 代码、SQLite、ACP 和 stdio MCP；不安装 claude-mem 的服务、Bun worker、Chroma 或插件 hooks。

[Dot 的任务与记忆说明](https://learn.chatgpt.com/docs/dots/tasks-and-memory) 支持持续对话、独立任务和持久记忆这一产品方向。Relay 没有复刻未公开的 ChatGPT 后端实现，也不推断其 session 轮换策略。

| claude-mem 中的职责 | Relay 原生实现 |
| --- | --- |
| 生命周期 hooks | Relay 发送前与 ACP Ready / TurnEnded / Error 事件 |
| 原始事件 | `sources` + `source_versions`，对话和工具证据 |
| 异步提炼 | SQLite `jobs` 队列 + 内部 observer ACP session |
| observation / summary | 带来源版本的 `memories`，类型包括 observation、summary、fact、decision、preference、working |
| 检索后按需展开 | `memory_search` → `memory_timeline` / `memory_get` → `memory_source` |
| 会话关联 | 派生的多对多 `topics` / `memory_topics`，不建立 Project 容器 |

## 存储和采集

`relay-runtime/src/native` 拥有 SQLite schema、迁移、采集、检索、工具和调度。`relay.sqlite3` 使用 WAL、FULL 同步、外键和事务。文件权限为 0600。UI 读取缓存快照，后台定期按数据库修订刷新，不在渲染时查询 SQLite。

会话文件仍采用临时文件、sync、rename 保存到 `threads/<id>/conversation.json`。发送前先保存待发送状态，再提交原始用户消息来源；来源无法落盘就不发送 ACP prompt。收到轮次终态时保存对话并采集用户/助手内容、完成状态和工具输入输出。ACP 工具证据各保留最多 16,000 字符，超出部分标记截断；上游未提供的内容不作补造。

源文本分为最多 24,000 字符的片段，保留完整对话正文；分片来源带稳定 key、hash 和版本。同一事件重放不会重复入队。对话文件是修复日志：后台扫描会补录崩溃发生在“对话保存成功、来源尚未写入”之间的事件。恢复不重发原用户任务。

历史 Codex 会话继续由后台归档服务只读导入。记忆扫描器每轮最多处理一个变化文件，完整扫描后至少间隔 60 秒再扫描。每 16 条可见消息形成 episode，再按大小分片；索引不截掉长消息尾部。单个归档当前上限 128 MiB，超过时保留原文件并记录诊断。原生工具输出未由历史解析器导入，只有 Relay 新收到的 ACP 工具证据进入此链路。

历史快照改写时，新版本使旧派生记忆失效；被移除的尾片段退出索引和队列，但保留版本记录。`owned_sessions` 记录 Relay 自己创建过的所有 ACP session，扫描器跳过它们；observer 工作目录也单独排除，防止把提炼过程再次当作新资料。

## 后台提炼

后台 observer 使用单独的内部逻辑线程，不出现在用户任务列表。每次任务使用干净的执行上下文，只向它暴露 `memory_commit_job`。它使用主 Agent 选择的本地 Codex 和模型偏好，并强制请求适配器的 `read-only` 模式，在独立空工作目录运行；后台不接受交互式工具授权。此模式属于 Harness 的权限边界，MCP scope 不是操作系统沙箱。

主 Agent 已连接后才启动提炼。默认每小时最多 60 次模型调用，每次只处理一个来源片段；实时 Relay 事件优先于历史导入。`memory_settings` 可暂停或设置 0–240 次/小时，`memory_status` 查看待处理、失败和已完成数量。暂停不影响采集与检索。推理仍通过用户选择的 Harness 及其模型服务发生，历史材料的提炼也会消耗该服务额度。

领取 job 使用事务与 600 秒租约，带递增 `attempt`。执行超过 480 秒结束本次提炼，最多尝试三次；连接失败有退避。提交同时校验 job、attempt、来源当前版本和租约。最多 12 条 observation 与 job 确认在同一事务中提交；一条不合法就全部回滚。迟到的旧 attempt 不可提交。工具已确认的重复提交不新增记录。

若 Harness 拒绝 observer 的 MCP 写入，它可以返回同 schema 的 JSON；ACP 会合并 commentary 和 final 文本，worker 只解码尾部完整 JSON 对象，经过相同的 job/attempt/来源校验提交。后台内容不能创建任务、确认事实或覆盖已有记忆。空 notes 表示该片段没有值得保留的内容，也是成功处理。

## 记忆、检索与主题

每条记忆有类型、scope、状态和一条或多条来源引用（`source_id` + `revision`）。后台结果强制为 `candidate`；主 Agent 依据用户实际决定和来源调用 `memory_write` 确认或通过 `supersedes` 修订。用户也可在记忆页面确认或修订；两种操作均创建保留原证据的新记录，旧记录变为 superseded。提议、失败执行与不确定结论必须在正文中保留相应语义。确认是 Agent 的有来源记录，不等于外部事实核验。

scope 为 null 表示共享；任务 Agent 只能写本任务 candidate，读取共享或本任务记忆。主 Agent 可以跨会话检索和整理。原始来源另外检查范围，任务不能借 source ID 读取其他任务的原文。来源改写会使候选和已确认派生记忆变为 stale，并同步撤下 FTS 索引；后续写入必须带当前来源版本。

FTS5 trigram 支持中英文检索；不足三个字符使用字面包含匹配。`memory_search` 返回最多 12 条紧凑索引，可选搜索原始来源。`memory_get` 每次最多 10 条、总正文预算 32,000 字符；timeline 返回同一来源组的相邻事件；`memory_source` 按 12,000 字符分页读取证据。每轮同时提供最多 8 条已确认偏好的紧凑条目，使语言和表达偏好不依赖本轮关键词命中。不会把全部历史注入每次 prompt。

observer 使用主题标签进行跨 session、多对多归类，并获得已有标签以便复用。`memory_topics` 列出集合；指定 topic 后返回记忆索引及关联 session。主 Agent 可用 `memory_merge_topics` 合并同义标签。此版是模型主题标签 + 本地全文检索，没有向量 embedding、自动层次聚类或专门的聚类图 UI；未提炼的历史仍可原文检索。

`memory_forget(memory_id)` 删除该记忆的正文并撤下索引。`memory_forget(source_id)` 同时清除该来源各版本正文、派生记忆及 job，保留 tombstone 阻止同来源重新导入。原始对话档案保持不变；这不是删除所有历史聊天文件的接口。其他独立来源中重复出现的同一事实需要分别处理。

## 任务一致性

`cancel_task` 可异步取消明确任务的排队或在途请求，取消只绑定相应 assistant ID；确认终态前不宣称停止。`create_task` 和 `continue_task` 要求稳定 `request_key`；相同 key 的相同操作幂等，不同输入冲突报错。任务创建、初始请求与目录修订同事务提交。worker 最多调度四个执行请求，主对话忙时结果通知排队。后台记忆 observer 单独串行。

worker 在发送前记录 `dispatching`，发送后绑定准确的 assistant message ID。只观察该回复的协议终态，不能拿后续无关消息充当结果。成功执行进入 `review`；主 Agent 审阅后才可标记 `completed`。执行结果和回到主对话的 outbox 请求在一个事务中落盘，唯一 key 防止重复通知。任务详情内的直接追问也进入同一结果回收路径。

等待授权/登录时任务显示等待状态，可打开任务处理；连接失败成为需要处理状态，不无限重连。应用重启后未确认派发/正在执行的请求标记 interrupted，检查记录后显式继续。连接中但尚未发送的请求可重新排队。常驻 worker 持有操作系统文件锁，第二个进程不能接管正在运行的队列。

## 迁移与验证

首次启动读取旧 `projects.json`（v1–v3）与 `projects/<id>`，旧定义变为历史线程，旧记忆作为待提炼的来源；主对话始终新建为 0。也会发现缺少目录项但仍有 conversation.json 的旧会话。原文件保留，复制后的执行 session/checkpoint 重置，防止旧项目角色进入新的常驻主对话。标记只在迁移完整成功后写入，重复执行幂等；不识别的格式报错保留。

新机制不需要 Jev/OpenRouter 密钥。旧配置与钥匙串条目不会被主动删除，也不会用于新输入。原有个人文件迁移继续读取旧 `projects/*/workspace`，文件空间不按任务拆分。

回归覆盖幂等投递、准确回复关联、结果 outbox、连接失败、未知投递恢复、租约 fencing、批次原子性、来源版本、长历史与尾片段失效、中英检索、scope、遗忘 tombstone、主题集合、提炼预算、MCP 握手和 ACP 新建/恢复时的 MCP 配置。完整检查使用 `cargo xtask verify`。

2026-10-04 已完成包边界、格式、全目标 Clippy 和 160 项测试检查。隔离数据目录中的真实 Codex 验证已跑通确认记忆写入与检索、异步任务执行、结果回到主对话、observer 候选提炼及来源记录。GPUI 的首页和活动页已在 1440×900、960×640 预览中检查。本次验证未启动用户生产数据目录的迁移。

## 记忆页面（2026-10-05）

独立的旧“本地会话”Tab 已替换为记忆页面，包含记忆、主题和来源视图。移除手动归属、按任务筛选、工作目录自动关联。归档服务保留，原文从来源详情打开。

页面读取真实队列与记忆数量，不把已导入、已处理、候选或已确认混为同一指标。显示连接等待、提炼暂停、小时额度耗尽、observer 连接错误和来源级失败信息；失败任务可手动重试，仍受暂停与额度限制。页面搜索使用字面正文/标题匹配，Agent 工具继续使用 FTS5；两者都不提供向量语义搜索。

记忆与来源列表每页 25 条，主题列表最多 80 个。来源按 12,000 字分页，完整 JSON 片段可转换为角色/正文展示，跨片段的原始材料仍可从归档查看。原始会话每页 40 条，支持翻页与单条全文复制。

本次 PR 覆盖入口与管理流程；不宣称完整对齐 claude-mem。Embedding、向量索引、混合召回和语义聚类仍是明确缺口。

2026-10-05 验证：`cargo xtask verify` 通过包边界、格式、全目标/全 feature Clippy（`-D warnings`）与 167 项测试。新增覆盖真实队列计数、空提炼结果、失败重试与暂停、用户修订及过期证据拒绝、分页、跨会话标签、遗忘、旧归属元数据兼容、筛选异步竞争与编辑草稿保留。

GPUI Metal 无窗口预览检查了 1440×900 和 960×640 的中英文记忆详情、修订、遗忘弹窗、来源、原文与主题页面，以及“已导入但零记忆”的空状态。预览使用固定测试数据，不代表用户历史已经提炼完成。复现命令为 `cargo run -p relay-ui --example quick_preview --locked -- /tmp/relay-memory-preview --workspace --memory-only`，可追加 `--english` 或 `--pending`。

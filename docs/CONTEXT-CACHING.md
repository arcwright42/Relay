# thread 上下文与缓存

更新：2026-10-04。thread 资料保持稳定快照/增量交付，原生记忆改为紧凑索引和工具检索。Project / Jev 归属和将全部薄记忆塞进每轮输入的方案已经替代，见 [原生机制](NATIVE-MEMORY.md)。

## 资料交付

`relay-runtime::context` 生成按 ID 排序的 thread 指令、选用文字资料与个人文件目录快照。thread 修改携带预期版本，保存成功才发布；修改在下一轮生效，不改变在途输入。

首次发送追加完整快照；后续比较该执行 session 已确认的快照，无变化不重复资料，有变化追加修改或移除 ID。稳定序列化和内容 hash 不包含随机数、当前时间、语言或连接状态。兼容读取旧 `project_id` 检查点字段。

发送前持久化未确认状态，保存失败不派发。正常轮次结束确认该轮发出的版本；拒绝、失败、取消或断连保留不确定状态，下次用户发送时重新追加当前资料快照。不自动重发原用户任务，ACP 也不提供上下文 exactly-once 回执。

原生 session/load 成功保留检查点；新建 session 或恢复失败重建快照并补入有限近期可见历史。逻辑 thread 身份不随执行 session 重建改变。模型配置切换不改写历史。

## 运行上下文与记忆

native runtime 在后台准备 prompt 时加入角色职责、逻辑 thread ID、有限任务状态和相关记忆索引。普通输入前保存当前用户消息来源，让 Agent 可以提交带 `source_id` 和 `revision` 的记忆。调用 `memory_search`、`memory_timeline`、`memory_get` 和 `memory_source` 逐层读证据，不注入全部历史。

候选、已确认、过时和已替代记忆有明确状态。旧来源或遗忘内容退出检索，但从索引撤下不等于删除 Harness 已经收到的历史。原生记忆的生命周期与资料同步检查点分开。

observer 使用独立、每次重置的执行上下文；任务使用独立 session 和有范围限制的工具。历史会话提炼不会改变主对话消息目标。

## 缓存边界和观测

Relay 的检查点只表达交付过的资料版本，不证明服务端仍有推理缓存。缓存 TTL、模型前缀、工具定义与原生压缩由 Harness 和模型服务管理；不承诺跨模型共享缓存，也不为维持 TTL 自动发请求。

首字延迟从接受发送到首个非空 assistant 文本；总耗时到 ACP 终态，包含工具和等待授权时间。记录模型、资料交付方式/版本、上下文字节数、补入历史与可选 token 用量。native 运行上下文计入字节数，即使资料交付方式为 unchanged，仍可有运行上下文和检索索引。

固定 codex-acp 1.12.0 的 usage 来自最近一次模型请求，inputTokens 不含 cachedInputTokens。缓存读取比例按 `cachedRead / (input + cachedRead + cachedWrite)` 展示，缺失与零分开；不是整轮用量或本地推算命中率。其他 Harness 接入前应单独校验计数语义。

验证使用确定性 payload、增量/移除、异常恢复、旧连接 fencing 和 ACP 子进程协议测试。完整检查：`cargo xtask verify`。

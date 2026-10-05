# thread 上下文与缓存

更新：2026-10-05。thread 资料保持稳定快照/增量交付，记忆由 Claude-Mem Provider 提供上下文与工具检索。Project / Jev 归属和将全部薄记忆塞进每轮输入的方案已经替代，见 [Memory Provider](MEMORY-PROVIDERS.md)。

## 资料交付

`relay-runtime::context` 生成按 ID 排序的 thread 指令、选用文字资料与个人文件目录快照。thread 修改携带预期版本，保存成功才发布；修改在下一轮生效，不改变在途输入。

首次发送追加完整快照；后续比较该执行 session 已确认的快照，无变化不重复资料，有变化追加修改或移除 ID。稳定序列化和内容 hash 不包含随机数、当前时间、语言或连接状态。兼容读取旧 `project_id` 检查点字段。

发送前持久化未确认状态，保存失败不派发。正常轮次结束确认该轮发出的版本；拒绝、失败、取消或断连保留不确定状态，下次用户发送时重新追加当前资料快照。不自动重发原用户任务，ACP 也不提供上下文 exactly-once 回执。

原生 session/load 成功保留检查点；新建 session 或恢复失败重建快照并补入有限近期可见历史。逻辑 thread 身份不随执行 session 重建改变。模型配置切换不改写历史。

## 运行上下文与记忆

runtime 在后台准备 prompt 时先加入 `<relay_runtime>` 角色职责、逻辑 thread ID 和有限任务状态，再按选中 Memory Provider 取得 `<relay_memory>`。注入可设为 `disabled / session / turn`，见 [Memory Provider](MEMORY-PROVIDERS.md)。发送前先把生命周期事件持久记录到投递队列，再交给独立 Claude-Mem Worker；没有 Relay 内部 Observer 或来源 ID 注入。MCP 工具清单也由同一 Provider 提供。

观察是未经用户确认的参考材料。删除 observation 不等于删除原始 prompt、tool event、总结或 Harness 已收到的历史。记忆投递生命周期与资料同步检查点分开。

Claude-Mem 的提炼 session、观察与总结由它的 Worker 管理。任务使用独立 session 和有范围限制的工具。历史会话提炼不会改变主对话消息目标。

Provider 配置指纹参与执行 session 的恢复标识，并传给 MCP 子进程核对；配置变更后重启 Relay，不能把另一 Provider 的工具与旧检查点混用。记忆上下文失败会显式提示，不能静默用另一个引擎替代。

## 缓存边界和观测

Relay 的检查点只表达交付过的资料版本，不证明服务端仍有推理缓存。缓存 TTL、模型前缀、工具定义与原生压缩由 Harness 和模型服务管理；不承诺跨模型共享缓存，也不为维持 TTL 自动发请求。

首字延迟从接受发送到首个非空 assistant 文本；总耗时到 ACP 终态，包含工具和等待授权时间。记录模型、资料交付方式/版本、上下文字节数、补入历史与可选 token 用量。Relay 运行上下文计入字节数，即使资料交付方式为 unchanged，仍可有运行上下文和检索索引。

固定 codex-acp 1.12.0 的 usage 来自最近一次模型请求，inputTokens 不含 cachedInputTokens。缓存读取比例按 `cachedRead / (input + cachedRead + cachedWrite)` 展示，缺失与零分开；不是整轮用量或本地推算命中率。其他 Harness 接入前应单独校验计数语义。

验证使用确定性 payload、增量/移除、异常恢复、旧连接 fencing 和 ACP 子进程协议测试。完整检查：`cargo xtask verify`。

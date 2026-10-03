# Relay 技术架构

状态：工作台、Codex 接入、真实项目与文字资料、项目记忆存储、上下文增量同步、对话诊断和本地语音唤醒已实现；Agent 记忆写入、任务线程、文件导入、检索和委派仍为目标架构。实现范围更新：2026-10-02。

## 1. 已确定的架构边界

- Relay 的 UI 和核心均使用 Rust 实现，macOS 优先。
- UI 采用 GPUI + GPUI Kit 的 Rust 组件层。
- Relay 作为 Agent Client Protocol（ACP）客户端连接 Agent。
- 项目拥有长期上下文；Agent 连接及执行会话可以更换。
- 首页 prompt 先通过独立 RoutingService 调用 Jev 判断项目归属，再进入项目执行；明确选择的项目不重新分配。
- 主 Agent 做任务规划、选择执行 Agent、审阅和整合。
- Relay 提供项目数据、调度工具、执行管理与桌面交互。

```text
划词 / 快捷键 / 主窗口 / 语音 / 视觉
                 │
           Relay 项目工作区
         ┌───────┴────────┐
   项目上下文服务       Rust ACP Client
   资料·记忆·决策           │
   历史·任务·成果       项目主 Agent
         │                 │ 调度工具
         └───────┬─────────┘
           Relay 任务运行管理
                 │ ACP
         ┌───────┼────────┐
       Agent A  Agent B  Agent C
         └───────┼────────┘
          结果 → 主 Agent 审阅
                 │
          归档到同一个项目
```

图中的主 Agent 与执行 Agent 均通过 Relay 的 ACP 客户端连接。调度工具是主 Agent 请求 Relay 执行派发的接口。

## 2. Rust 模块划分

以下区分当前包边界与长期职责规划。UI 框架为 GPUI + GPUI Kit，当前依赖均固定版本。

当前 workspace 包含 `relay`（应用入口及装配）、`relay-ui`（视图与交互）、`relay-core`（领域数据与服务接口）、`relay-platform`（macOS 快捷键及辅助功能 FFI）、`relay-runtime`（安装、网页抓取、运行状态、对话存储）、`relay-acp`（SDK、进程与协议边界）和 `xtask`（开发工具）。UI 只依赖领域接口，由入口注入 runtime。后续模块有具体实现时再拆包，依赖方向与检查规则见 [包治理](PACKAGES.md)。

| 模块 | 职责 |
| --- | --- |
| desktop | 窗口、菜单栏、快捷面板、浮层和交互事件 |
| platform | 各操作系统的选区、窗口、快捷键、截图与音频能力 |
| projects | 项目目标、指令、归属与主 Agent 角色绑定 |
| context | 资料、记忆、检索、来源、版本和任务上下文快照 |
| tasks | 任务记录、依赖、执行尝试、事件收件箱、并发控制与恢复 |
| acp-client | 进程连接、协议协商、会话、流式事件、取消与能力检查 |
| agent-registry | ACP Agent 启动配置、版本、认证状态和能力记录 |
| agent-tools | 向主 Agent 提供项目读取、派发、跟进与记忆整理工具 |
| storage | 本地数据库、附件与产物索引 |
| voice | 转写、朗读、唤醒与视觉输入的轮次关联 |

当前对话按项目使用有格式版本的 JSON 文件，以临时文件、sync 和 rename 替换保存；流式回复定期检查点，退出时刷新。无法读取的文件保留原样，阻止覆盖。完整项目资料库实现时再引入数据库和附件索引。Rust 核心与 UI 渲染解耦；系统能力未来通过 Rust 平台模块封装。

界面语言属于应用级偏好，通过独立 `SettingsService` 接口由 runtime 持久化到 `settings.json`。默认简体中文，可在设置中即时切换 English；UI 文案表和组件 locale 负责显示，项目内容、Agent 连接及协议值不随语言变化。

语音通过 `VoiceService` 与现有 Agent、项目服务并列。`relay-core::voice` 拥有会话/轮次快照和采音、ASR、项目对话、朗读端口；`relay-runtime::voice` 分别管理监听生命周期、资源校验、轮次取消与 Prompt 路由执行；`relay-platform::voice` 持有 macOS 权限、CPAL、sherpa-onnx KWS/Silero VAD 和流式 PCM 播放。原生 SDK 留在平台包，标准 WAV 领域载荷交付 runtime；云端 TTS 通过 PCM 播放端口回到平台层。应用入口完成依赖注入并管理独立语音浮层，UI 只操作领域服务。

链路为「嘿 relay / Hey Relay → VAD 连续静音约 0.8 秒 → ASR → Prompt → Jev → 项目 Agent session → TTS → 继续监听」。处理和播放阶段暂停收音，用户可取消本轮重说；语句超过 60 秒拒绝提交并提示重说。Jev 不确定时选择项目，Agent 需要授权时在项目会话中确认。派发返回 assistant message ID，所有回复和取消绑定本轮，防止读到其他窗口的回复。唤醒和 VAD 模型在本地进程内运行；完整语句由千问 ASR 转写，回复由千问 TTS 合成，音频不落盘。`cloud` 模块拥有配置、Keychain、WebSocket、ASR 和 TTS 适配，UI 不持有密钥。未配置密钥时不会上传或执行任务。具体资源和生命周期规则见 [包治理](PACKAGES.md#语音唤醒领域)。

`RoutingService` 与 `AgentService` 并列：runtime 的 Jev HTTP 客户端返回已有项目、新项目或用户选择；UI 接收结果后校验项目版本，使用 `CreateAtRevision` 保存新项目，再触发既有 Agent 连接和发送链路。Jev 不经过 ACP、不管理会话、不调度执行工具。API key 通过 macOS Keychain 独立管理。详见 [Jev 项目归属方案](PROJECT-ROUTING.md)。

会话层级确定为 `Prompt → Jev → Project → 主对话 → 按需任务线程`。项目主 Agent 复用现有 ACP 会话，不新增主 Agent 服务，也不把每次项目内输入交给第二次 Jev。未来任务线程有独立会话绑定，共享项目记忆，并可接纳导入的已有 Harness 会话。当前仍是一项目一主对话，线程、导入和委派没有在本次记忆实现中引入。

## 3. ACP 接入

ACP 定义客户端与 Agent 的双向请求和事件通知。Relay 实现客户端职责，优先以本地子进程和 stdio 建立连接。[ACP 协议概览](https://agentclientprotocol.com/protocol/v1/overview)

接入路径：

| Agent | Relay 面向的 ACP 入口 | 说明 |
| --- | --- | --- |
| Codex | codex-acp 适配器 | 由适配器处理其内部运行时协议 |
| Claude 系 | claude-agent-acp 适配器 | 连接 Claude Agent SDK 的 ACP 实现 |
| OpenCode | opencode acp | 使用其 ACP 子进程入口 |
| 其他 Agent | 兼容的 ACP 入口 | 按协商能力启用功能 |

当前仅实现 Codex，使用本地安装，已完成模型发现和真实消息验证；其余为未来路径。[Codex ACP](https://github.com/agentclientprotocol/codex-acp)、[Claude ACP](https://github.com/agentclientprotocol/claude-agent-acp)、[OpenCode ACP](https://opencode.ai/docs/acp/)

Codex 采用内置入口与本地可执行文件发现，Relay 不安装或升级 Harness 本体。连接链路为 `GPUI → AgentService → relay-runtime → relay-acp → stdio → codex-acp → Codex`。Rust ACP SDK 固定 2.2.0，协议先协商 v1。上游适配器使用 JavaScript，Node 与适配器是单独管理的外部组件；npm 安装省略可选平台 CLI 二进制，启动时始终指定用户本地 CODEX_PATH；Relay 的 UI、领域、安装器、状态机和协议客户端均为 Rust。

每个项目独立维护连接、工作目录、原生会话引用、可见消息与已确认的配置偏好。模型及其他选择项来自 `configOptions`；请求确认前禁止重复切换，不猜测模型 ID。后台工作线程负责安装、协议 I/O 和持久化，UI 只订阅快照修订。连接 generation 防止旧进程的迟到事件改变新会话。项目历史恢复与原生 session/load 分开处理，抑制原生历史重播带来的消息重复。详见 [Codex 接入](CODEX.md)。

适配器可能包含自己的运行时依赖。使用用户已有二进制还是适配器配套版本，需要在连接设置和兼容性验证中明确。ACP 连接也不代表自动获得订阅、图像或恢复能力。

### 协议版本

初始化时协商版本与能力，锁定已验证的组合。官方当前同时提供 v1 和 v2 文档，生命周期存在区别：v1 的 prompt 响应表达轮次结束；v2 的 prompt 响应可先确认接收，后续状态更新表达结束。协议处理须按版本隔离，不能混用终态判断。[ACP v1](https://agentclientprotocol.com/protocol/v1/overview)、[ACP v2](https://agentclientprotocol.com/protocol/v2/overview)

会话恢复、图像、音频、内嵌资源、文件与终端相关能力，按目标版本和运行时实际能力启用。上下文传递可以使用协议支持的文本、资源和图像内容块。[ACP 内容类型](https://agentclientprotocol.com/protocol/v1/content)

## 4. 上下文与执行分离

项目存储是资料与共享记忆的权威来源。ACP session ID 只是执行连接的引用，不能充当项目 ID。

当前项目、文字资料与记忆保存于独立的 projects.json，通过 ProjectService 编辑并校验预期版本。每个执行会话保存已确认的上下文快照和同步检查点：首轮追加按资料和记忆 ID 排序的快照，后续仅追加变化。正常请求不重写项目开头，不重复注入历史。原生恢复沿用检查点；新会话重新生成快照。发送前持久化不确定状态，失败/拒绝/取消后在下一条用户消息上重发当前快照，不自动重发用户任务。具体结构、大小限制、用量口径和验收见 [上下文与缓存方案](CONTEXT-CACHING.md)。

薄版记忆由 `relay-core::MemoryItem` 定义：项目内单调 ID、`Fact / Decision`、标题、正文、可选 `MemorySource::Message`。来源 ID 指向当前项目的 Relay 可见消息，不使用 ACP session ID；编辑保留来源。保存复用项目写锁、乐观版本检查和原子替换，不新增依赖、服务或数据库。项目目录格式 v2 兼容读取 v1，首次成功修改才升级；旧版应用拒绝覆盖 v2，避免丢失记忆。旧会话快照缺少 memory 字段时按空记忆读取，下一轮可正常生成增量。

项目主 Agent 负责维护事实与决策，Relay 负责存储及上下文交付；会话 UI 已移除手动记忆管理与回复转存入口，Agent 写入接口仍待接入。所有保留的记忆随项目上下文交付，Jev 只接收最近新增的最多三个标题片段，不接收记忆正文和来源。事实/决策分类不代表 Relay 自动验证内容；向量检索、去重和跨项目共享后续扩展。

Relay 的检查点表示已交付的项目版本，不能证明模型缓存仍有效。缓存标记、工具定义与原生上下文压缩由 Harness 管理；项目共享资料不保证跨模型、目录或 Harness 共享推理缓存。

| 实体 | 归属与含义 |
| --- | --- |
| Project | 长期目标与上下文的容器 |
| ContextItem | 项目材料及其来源、版本、内容引用 |
| MemoryItem | 当前已实现的项目事实/决策、正文和可选回复来源；修订使用项目版本 |
| Conversation / Thread | 项目主对话或任务执行线程，保存可见消息 |
| Task | 主 Agent 定义的目标、范围、依赖和验收要求 |
| Run | 某次执行尝试，绑定 Agent、上下文快照和运行状态 |
| ContextSnapshot | 本次执行所使用的项目材料及记忆版本 |
| AgentConnection | ACP 入口、认证方式、版本与能力 |
| SessionBinding | Run/对话与 ACP 会话的对应关系 |
| Artifact | 项目产物，保留生成任务和引用材料的来源关系 |

Agent 可以是某项资料的创建者，但创建者信息不改变该资料归项目所有的事实。

每次委派形成一个上下文快照，包含任务目标、约束、相关记忆、材料引用、输出要求与基线版本。执行 Agent 需要更多材料时，通过受范围约束的项目读取工具获取。

项目持续积累资料，单次模型输入仍然有大小限制。初始采用明确引用、摘要和项目内检索，索引方式根据数据量再选择。

执行结果先作为产物或候选结论归档，主 Agent 审阅后提交记忆更新。记忆更新附带来源和基线版本，并发冲突不能用最后一次写入静默覆盖。

## 5. 主 Agent 如何调度

主 Agent 通过工具请求 Relay 创建任务、选择 ACP Agent、读取项目材料和取得执行结果。建议工具如下，名称为 Relay 设计，属于应用能力：

- project.search / project.read：读取当前项目材料。
- task.delegate：提交任务目标、执行 Agent、上下文引用和产物要求。
- task.status / task.results：查询进度和结果。
- task.send_input / task.cancel：补充要求或请求取消。
- project.update_memory：提交带来源和版本的共享记忆变更。

建议将这些工具通过本地 MCP 服务提供给主 Agent，并通过 ACP 的会话配置关联。ACP v1 会话设置支持传入 MCP 服务配置；具体接入需验证目标 Agent 的实现。[ACP 会话设置](https://agentclientprotocol.com/protocol/v1/session-setup)

**ACP 负责 Relay 与 Agent 的会话通信；项目上下文服务与主 Agent 调度工具负责跨 Agent 工作协作。** 原生子 Agent 的专有能力可作为增强，基础方案采用 Relay 管理的独立 ACP 会话。

调度工具返回持久化任务标识。任务完成后，Relay 将结果加入主 Agent 的项目事件收件箱；在其会话可以接收输入时通知或继续对话，避免在活动轮次中随意插入第二次 prompt。

主 Agent 决定拆分、重试策略与结果取舍。Relay 校验归属、维护队列、执行并发限制、记录依赖和处理资源清理。

## 6. 替换 Agent 与恢复

- 更换执行 Agent：保留项目、任务和历史，创建新的 Run 与 ACP 会话，从上下文快照和已完成结果继续。
- 更换主 Agent：恢复项目目标、记忆、任务列表、执行中状态和未处理结果；已有执行线程继续受 Relay 管理。
- 主 Agent 角色绑定带版本，旧绑定产生的迟到调度请求不得改变新主 Agent 已接管的任务。
- Agent 原生会话可恢复时优先恢复；否则从项目记录创建新的执行上下文，并记录此次恢复方式。
- Agent 隐藏状态和未持久化内部推理不承诺迁移。

## 7. 执行一致性

事件携带 project、task、run 和 session 的关联标识。每次执行尝试单独记录，避免断线后重复派发有副作用的工作。

取消先进入取消中状态，继续接收必要的工具终态，再根据协议确认结束。连接断开且结果未知时保留未知状态，恢复后核对。

上下文共享不等于所有任务并发改同一份文件。写入代码的并行任务建议使用隔离工作目录或 worktree，主 Agent 审阅并整合结果；共享记忆由版本化接口写入。

项目读取工具按绑定项目与任务范围提供材料。文件与执行权限由对应运行环境落实，ACP 本身不能替代文件系统隔离。

## 8. 桌面与多模态输入

系统选区、浏览器扩展、文件、截图、音频转写均先成为 Relay 的上下文项，再按用户动作加入项目或提交主 Agent。

macOS 候选路径为辅助功能 API 获取选区和窗口信息；目标应用暴露的数据与事件需实测。浏览器扩展作为网页兼容性路径，通过原生通信进入 Rust 核心。[Apple 选区属性](https://developer.apple.com/documentation/applicationservices/kaxselectedtextattribute)、[浏览器原生通信](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)

选区快照应在浮层获取焦点之前完成。来源无法获取时保持缺失，选区失败时不使用旧内容。多屏定位、页面缩放、PDF 和富文本分别验收。

语音已按“本地端点检测 → 千问 ASR → Jev → 项目 Agent → 千问 TTS”编排；有图像能力时将同一轮截图一并发送。唤醒词和持续窗口共享作为可开启、可结束的模式，独立管理输入生命周期。

快捷入口按最新决定提前到多 Agent 委派之前：Relay 平台采集选区及可取得的 URL，runtime 后台调用 Moli，UI 发送时冻结已取得的材料。抓取不阻塞提交，迟到结果不改变本轮输入。见 [快捷入口方案](QUICK-ENTRY.md)。

## 9. 建议实施顺序

| 步骤 | 产出与验证 |
| --- | --- |
| 0 | GPUI + GPUI Kit 工作台壳子，验证窗口、布局、项目切换和输入交互；当前暂不展示右侧任务栏 |
| 1 | Rust 项目数据模型、独立上下文存储和 ACP 握手验证 |
| 2 | 项目主对话，验证材料保存、读取、流式回复和重启恢复 |
| 3 | 主 Agent 调度工具与独立执行会话，验证任务派发和结果回收 |
| 4 | 主 Agent 更换与第二种 ACP Agent 接入，验证上下文不依赖某一运行时 |
| 5 | 网页划词、快捷面板和项目主窗口的同一工作链路 |
| 6 | 按键语音、朗读、截图问答，再扩展唤醒与持续视觉 |

当前已完成工作台、Codex ACP 主对话、可见历史恢复、真实项目与文字资料、薄版项目记忆、Jev 项目归属、上下文快照/增量和诊断，以及快捷面板的选区/网页上下文基础实现；本地 Codex 会话中心按 30 分钟归档外部对话并支持项目归属，详见 [本地会话中心](CLIENT-SESSIONS.md)。任务线程、其他 Client 导入、原生续聊、文件导入、检索、委派和语音视觉按上述目标继续实现。

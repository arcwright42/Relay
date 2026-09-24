# Relay 开源产品对照与能力规划建议

调研日期：2026-09-22。Relay 基线：`e549916`。本文是规划建议，尚未替代已确认的产品规格或启动功能开发。

范围按 Relay 现有定位确定：macOS 优先、Rust + GPUI、通过 ACP 连接本地 Agent，项目拥有上下文，覆盖研究、写作、研发等电脑使用场景。证据来自官方文档、公开仓库和选取的实现文件；没有逐个安装验收，也没有将默认分支代码等同于已发布版本或实测可靠性。

## 主要结论

下一阶段应优先完成 **桌面入口、项目内多线程、资料来源与成果归档**。这些能力能让现有 Codex 接入形成日常使用体验，也为主 Agent 委派打基础。

建议调整原实施顺序：把全局快捷面板与首批网页选区支持，提前到复杂多 Agent 调度之前；同时先解除数据模型中“一个项目对应一个执行会话”的约束。Codex 仍足够验证第一条完整工作链路。

Relay 值得强化的方向是：用户在任意应用遇到材料，可以直接交给 Agent，之后回到同一个项目继续工作；资料、决策和成果可独立于执行 Agent 保留下来。

## 当前能力核对

以下依据实际代码，而非仅依据目标架构。

| 能力 | 当前状态 | 依据及缺口 |
| --- | --- | --- |
| Codex 执行 | 已实现基础链路 | 托管安装/本地路径、ACP、认证、动态配置、流式回复、权限确认、取消与恢复。见 [CODEX.md](CODEX.md) |
| 项目与资料 | 已实现基础持久化 | 可创建项目、编辑指令、保存和选用文字资料；`ContextItem` 只有 ID、名称、内容和选用标记，缺少来源与附件类型。见 [projects.rs](../crates/relay-core/src/projects.rs) |
| 上下文连续性 | 已实现快照/增量及诊断 | 已有交付检查点与可选缓存用量，不能据此认定线上缓存已命中。见 [CONTEXT-CACHING.md](CONTEXT-CACHING.md) |
| 项目内多对话 | 尚未实现 | 连接与运行状态以 `ProjectId` 为索引，缺少独立 Thread/Task/Run。见 [runtime](../crates/relay-runtime/src/lib.rs) 与 [AgentService](../crates/relay-core/src/agents.rs) |
| 项目归属判断 | 可选 Jev 接口已接入 | 实际服务可用性依赖账号；当前可走手动选择。应保持它是增强能力，不成为开始工作的前提。见 [PROJECT-ROUTING.md](PROJECT-ROUTING.md) |
| 全局搜索、Inbox | 只有部分界面基础 | 搜索目前过滤项目名称；Inbox 仍为固定空态，尚未聚合执行事件。见 [导航](../crates/relay-ui/src/workbench/navigation.rs) 与 [工作台](../crates/relay-ui/src/workbench/mod.rs) |
| 图片、文件、MCP、委派 | 尚未形成 Relay 的能力链路 | ACP 发送目前只有文本内容，创建会话未传入 Relay 管理的 MCP 配置。Harness 自带工具不等同于 Relay 已管理这些能力。见 [ACP 实现](../crates/relay-acp/src/lib.rs) |
| 划词、全局面板、语音、视觉 | 已在产品目标中，尚未实现 | 需要平台采集、来源关联、浮层和对应生命周期。见 [PRODUCT.md](PRODUCT.md) |

## 最值得参考的项目

### Codeg：最接近的 ACP 工作台

官方文档描述了外部会话导入与检索、跨会话引用、主 Agent 委派、独立子会话与任务状态。委派通过内置 MCP 工具进入支持该能力的 Agent，提交后返回任务句柄；子任务只收到交付给它的材料，不自动继承主会话全部上下文。[会话聚合](https://docs.codeg.app/guide/aggregation) · [委派机制](https://docs.codeg.app/guide/multi-agent)

已检查其 Rust 委派类型与服务实现，包含父连接/父会话关联、任务标识、取消和等待用户的状态；仓库也有相应端到端测试文件。最值得借鉴的是任务和执行会话的组织方式、结果回收、等待授权时的可见性。[委派数据结构](https://github.com/xintaofei/codeg/blob/57bc49c4be123e327afdc9c33abb99d8df3a6233/src-tauri/src/acp/delegation/types.rs) · [测试文件](https://github.com/xintaofei/codeg/blob/57bc49c4be123e327afdc9c33abb99d8df3a6233/src-tauri/tests/delegation_e2e_uds.rs)

对 Relay 的建议：先实现项目内主对话和任务线程；委派首版只做一层、一个执行 Agent，并保留重启后的任务记录。Codeg 的完整 Git、终端、无限画布可留在参考范围，当前不列入 Relay 首版。

### Zed：ACP 客户端职责与能力边界

Zed 支持从 ACP Registry 安装外部 Agent、自定义入口、导入并恢复原生线程，以及查看 ACP 日志。文档明确区分客户端能力和外部 Agent 自己的认证、模型、原生配置与技能；MCP 是否转交、技能是否生效需要看实际集成。[External Agents](https://zed.dev/docs/ai/external-agents)

对 Relay 的建议：建立每个 Harness 的能力记录，界面据此开放图片、原生恢复、MCP、分叉等操作。下一种 Harness 接入应验证同一项目资料能否交接、哪些状态可以恢复，以及缺失能力如何呈现。保留现有按 Agent 返回值展示模型的做法。

### Cherry Studio：划词工具条与应用兼容性

其 README 把部分 Selection Assistant 增强列入路线图，但当前代码已经有实际的选区服务、浮动工具条、动作窗口、快捷触发、屏幕边界定位和应用过滤；macOS 路径检查辅助功能权限。应区分现有基础能力和后续增强。[产品仓库](https://github.com/CherryHQ/cherry-studio) · [SelectionService 实现](https://github.com/CherryHQ/cherry-studio/blob/3746e316fab67665219f4d92abeff22b35718f84/src/main/services/selection/SelectionService.ts)

对 Relay 的建议：先做快捷键主动触发，明确支持的应用范围，再补自动浮现。一次采集形成不可混淆的选区快照；工具条取得焦点之前捕获内容，失败时允许粘贴，保留来源应用、可取得的 URL 和目标项目。应用排除、快捷键冲突和多屏定位应列入验收。

### AionUi：文件成果与通用办公任务

官方仓库提供多 Agent 会话、MCP 管理、技能与定时任务等使用说明。其文件预览模块明确实现多标签与写入后更新，列出 Markdown、图片、PDF、Office、HTML 和 Diff 等查看器。[产品说明](https://github.com/iOfficeAI/AionUi/blob/main/readme.md) · [预览模块文档](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/conversation/Preview/README.en.md)

对 Relay 的建议：任务完成后应有可找到、可打开、可继续编辑的成果卡片，记录所属项目和生成任务。首批支持 Markdown、文本、图片及文件链接；Office 可先调用系统应用打开，按实际需求扩充内置预览。定时任务放在执行恢复与结果通知稳定之后。

### AnythingLLM：资料检索与可管理的记忆

官方仓库描述文档导入、工作区、检索与引用来源；记忆文档区分工作区记忆和全局记忆，并提供编辑、删除和作用域调整。自动提取与使用记忆有独立开关。[资料能力](https://github.com/Mintplex-Labs/anything-llm) · [记忆机制](https://docs.anythingllm.com/features/memories)

对 Relay 的建议：区分原始资料、项目事实/决策、个人偏好和本轮临时附件。先提供文件/网页导入、全文搜索、明确引用与可编辑的项目摘要；资料规模扩大后再评估语义检索。长期记忆记录来源及修订，避免把整段聊天或未经核实的 Agent 结论直接当作项目事实。

### Goose：可复用任务与扩展工具

Goose 的扩展通过 MCP 提供工具，Recipes 可以组合指令、参数、工具配置和结构化输出要求，并支持保存、编辑与重复运行。这些是官方使用文档描述的现有功能。[扩展](https://github.com/aaif-goose/goose/blob/main/documentation/docs/getting-started/using-extensions.md) · [Recipes](https://goose-docs.ai/docs/guides/recipes/session-recipes/)

对 Relay 的建议：把“总结网页”“比较资料”“整理会议记录”等操作发展为少量可编辑动作，声明所需材料、可用工具和期望成果。ACP 继续连接 Agent，MCP 向 Agent 提供 Relay 的项目读取、资料搜索与任务工具；技能的加载方式按 Harness 适配。先验证具体动作，随后考虑市场与插件分发。

### Handy：按键语音输入与 Rust 实现参考

Handy 支持按住/切换快捷键录音、本地语音识别并回填文本；默认文档描述 Whisper/Parakeet 等模型选项。检查的 Rust 协调器包含录音、处理中、按键释放、取消和重复触发的状态管理。[产品说明](https://github.com/cjpais/Handy) · [Rust 协调器](https://github.com/cjpais/Handy/blob/8f9cf53cd1410cda26beea39ff802ac306e39585/src-tauri/src/transcription_coordinator.rs)

对 Relay 的建议：先做按键说话 → 可修改转写 → 发送到当前线程，再加入朗读和打断；用户可以在同一轮明确附加截图。中文准确率、首字延迟、模型下载体积和资源占用需要在目标 Mac 上验证，不能依据支持语言的声明直接选定引擎。

### 辅助参考：Screenpipe 的许可边界

Screenpipe 的连续屏幕/音频采集和可检索上下文对产品研究有价值。但当前仓库明确使用 Screenpipe Commercial License，商业使用与集成受到限制，属于源码可见项目，因此不计入本次开放许可项目的可复用代码候选。此前 MIT 版本与当前版本应分别判断。[当前许可](https://github.com/screenpipe/screenpipe/blob/c460ff629a4e261a3a91448c0cfb2dc0402a91be/LICENSE.md)

Relay 可独立评估用户主动选择窗口/截图的交互，持续采集留待有明确需求后再规划。[产品的数据边界说明](https://screenpipe.com/security)

## 缺口与优先级

P0 指下一版的核心使用链路；P1 在 P0 稳定后逐项推进；P2 保留探索。优先级是针对 Relay 定位的判断，不是上游产品排名或工期承诺。

| 优先级 | 能力 | Relay 第一版应做到什么 | 与原规划的关系 |
| --- | --- | --- | --- |
| P0 | 全局入口与小窗 | 快捷键唤起、选区/粘贴卡片、选择项目、流式回答、展开后仍是同一线程 | 已规划，建议提前 |
| P0 | 项目内多线程 | 主对话之外可创建任务线程；重命名、归档、恢复；小窗与主窗口共享线程标识 | 已在目标架构中，建议先落实数据模型 |
| P0 | 有来源的上下文 | URL/文件/选区/截图具备类型、来源与内容版本；用户能看见本轮发送哪些材料 | 已规划，建议拆成可验收的小步 |
| P0 | 等待状态与 Inbox | 区分运行、待授权、待回答、失败、完成；跨项目聚合需要用户处理的事项 | 现有空态的具体产品化建议 |
| P0 | 成果卡片 | 保存生成文件/正文与任务关联，支持预览、打开、再次引用 | 已规划，建议避免结果仅留在聊天里 |
| P1 | 项目检索与记忆 | 搜索资料/历史/成果；可查看来源、编辑和恢复项目决策 | 已规划，先做全文搜索与显式引用 |
| P1 | MCP 与动作模板 | 项目级工具开关、连接检测、少量参数化动作；能力按 Harness 生效 | 已有工具架构，补可用的产品入口 |
| P1 | 第二个 Harness 与交接 | 优先评估 Claude ACP；同一项目切换后带上目标、材料、已定事项和未完成工作 | 验证上下文与具体 Agent 分离 |
| P1 | 主 Agent 委派 | 一个主 Agent 派发一个子任务，观察、取消、等待用户、恢复、回收成果 | 保留原方向，在多线程基础上实施 |
| P1 | 按键语音与单次截图 | 可见录音/转写，修正后发送；截图绑定本轮，按 Agent 能力传递 | 已规划，先于唤醒词和持续视觉 |
| P1 | 导出备份与已有会话导入 | 项目可导出恢复；导入旧会话有来源与去重，无法恢复原生会话时能看历史 | 新增明确的留存与迁移要求 |
| P2 | 定时、远程与复杂并行 | 有稳定任务状态、去重和通知后再做；按真实需求决定远程入口与并发规模 | 自动化扩展方向 |

对于 Inbox，建议沿用左侧现有入口，显示“待我处理”和任务结果；不以恢复原先右侧常驻任务栏作为实现前提。授权仍关联具体任务，通知点击后回到原线程。

## 建议下一版的完整使用场景

用户在浏览器中看到一段材料，选中后按快捷键，Relay 小窗显示这段材料及来源。用户选择已有项目，点击“解释”或输入问题，收到流式回答；展开工作台后继续同一条对话。材料能保存在该项目，回答或生成文件能作为成果再次引用。

落地次序：

1. 引入独立 Thread 与来源引用；旧项目历史迁移为默认主线程，保留原生会话绑定。
2. 快捷面板先支持文字粘贴与明确项目选择，完成小窗/主窗连续性。
3. 验证首批 macOS 应用的选区采集；Chrome、Safari、TextEdit 可作为候选验收样本，兼容性通过实测确定。
4. 加入网页来源和文件引用、材料列表及基础成果卡片；再扩展 PDF 文本提取与截图。
5. 接通任务状态、待处理事项与完成通知；把失败原因和恢复动作放在当前任务附近。

首页自动判断项目可以随后接入同一入口。显式选择项目和未分类材料的暂存应始终可用，首条工作链路不依赖额外分类服务的密钥。

验收建议：

- 小窗与主窗口的项目、线程、输入和流式结果一致；展开不重复发送。
- 选区采集失败或换了窗口后不使用旧材料，原应用焦点与剪贴板得到妥善处理。
- 同项目不同任务的草稿和执行会话独立，共享的是用户指定的项目材料。
- 重启后能找回资料、任务状态和成果；未知执行结果不能被当作成功，也不能自动重发有副作用的操作。
- 记录快捷入口耗时、采集成功率、首次回复时间和可恢复失败比例。首版先建立基线，不提前宣称性能改善。

## 建议纳入规划的产品细节

**本轮材料清单。** 发送前和回复后，都能看见引用了哪些网页、文件、片段和项目决策；可展开来源。当前已有上下文交付诊断，可以沿此方向发展成用户能理解的功能。

**项目交接摘要。** 更换 Harness 或重建会话时，提供目标、约束、已确认结论、待办和成果引用。用户可以检查摘要；Agent 的内部推理和私有运行状态不属于可保证迁移的数据。

**未分类材料暂存。** 用户只想先收下一段材料时，可以先保存再整理，避免每次采集都必须新建项目或发起推理。这可作为 Home 的轻量入口，与 Inbox 的待处理执行事件保持明确区别。

**诊断给出下一步。** 将安装/路径、登录、权限、额度、网络、服务端错误分别说明；区分本地准备、上下文处理与模型回复耗时。上一轮真实网关测试出现账号验证要求，正说明“连接失败”或“密钥错误”这样的统称不够准确。

**恢复与迁移可验证。** 项目导出涵盖资料、来源、历史和成果索引；恢复前有版本检查与预览。导入用户已有 Agent 会话时应由用户选择来源，不把“发现本地安装”等同于读取其全部历史。

## 与 Rust 和包治理的关系

保持现有依赖方向：`relay-core` 定义 Thread、Task、Run、SourceRef、Artifact 等需要落地的领域类型；`relay-runtime` 承担存储、检索和生命周期；`relay-acp` 做协议转换；`relay-ui` 展示领域状态；平台采集从独立 Rust 模块起步，有稳定边界后再决定是否拆 crate。

多线程意味着 AgentService 与连接表需要从仅按 ProjectId 管理，调整为明确的项目/线程/执行关联。项目资料继续归项目所有，执行会话绑定到线程或 Run。这一步优先于跨 Harness 委派。

Relay 提供的项目工具可通过会话级 MCP 配置交给支持它的 Agent，并限定项目与任务范围。工具传递、认证、技能和原生恢复逐项验证；ACP 连通不代表这些能力自动存在。此处与 [Zed 的外部 Agent 边界](https://zed.dev/docs/ai/external-agents)及 [Codeg 的委派接入条件](https://docs.codeg.app/guide/multi-agent)一致。

资料与线程数量增长后，再评估本地数据库与全文索引。当前调研不引入新依赖、外部向量数据库或插件运行时，也不改变 GPUI 的 UI 技术选择。

截至调研日，仓库声明的许可如下；实际引入代码时仍应核对具体文件和依赖，而非只看根目录标签。

| 项目 | 仓库声明 | 本次用途 |
| --- | --- | --- |
| [Codeg](https://github.com/xintaofei/codeg/blob/main/LICENSE) | Apache-2.0 | Rust ACP 与任务结构参考 |
| [AionUi](https://github.com/iOfficeAI/AionUi/blob/main/LICENSE) | Apache-2.0 | 文件成果与工作台体验参考 |
| [Goose](https://github.com/aaif-goose/goose) | Apache-2.0 | 工具和可复用任务参考 |
| [AnythingLLM](https://github.com/Mintplex-Labs/anything-llm) | MIT | 资料与记忆产品机制参考 |
| [Handy](https://github.com/cjpais/Handy/blob/main/LICENSE) | MIT | Rust 音频状态机参考 |
| [Zed](https://github.com/zed-industries/zed#licensing) | 主要 GPL-3.0-or-later，标注组件 Apache-2.0 | GPUI/ACP 客户端设计参考 |
| [Cherry Studio](https://github.com/CherryHQ/cherry-studio/blob/main/LICENSE) | AGPL-3.0 | 划词交互与兼容性参考 |

本轮只读取和比较了上游代码，没有复制其实现进入 Relay。

## 暂缓范围

完整 IDE（编辑器、Git 面板、终端）、无限画布、插件市场、多级 Agent 自组织、大量 Harness 同时接入、移动端/远程服务、持续录屏和全天语音唤醒，均暂不作为下一版主线。

语音唤醒、视觉交互和主 Agent 调度仍保留在产品方向中；先完成按键语音、单次截图和一层委派，验证价值与可靠性后逐步扩展。

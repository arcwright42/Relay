# Relay

<img src="assets/relay-icon.png" width="96" alt="Relay app icon" />

**一个面向本地 AI Agents 的桌面统一入口，让 Codex、Claude Code、OpenCode 等 Agent 可以在电脑上的任何场景被即时调用。**

Relay 以项目组织长期上下文，以主 Agent 协调工作，通过 Agent Client Protocol（ACP）连接具体 Agent，使用 Rust 实现。

## 已确定的产品原则

- **随处调用**：网页划词弹窗、全局快捷键、客户端对话、语音唤醒及视觉与语音交互。
- **项目持有上下文**：资料、历史、记忆、决策和成果归属于项目，与具体 Agent 分离。
- **主 Agent 调度**：用户与项目主 Agent 沟通；主 Agent 决定任务拆分、执行 Agent、上下文范围与结果整合。
- **Agent 维护记忆**：主 Agent 整理项目事实、决策和来源，Relay 负责存储与共享。
- **ACP 统一连接**：Relay 作为 ACP Client，连接原生支持 ACP 的 Agent 或兼容适配器。
- **Rust 实现**：UI、桌面核心、项目上下文服务、任务运行管理和 ACP 客户端均使用 Rust。
- **UI 框架**：采用 GPUI + GPUI Kit 的 Rust 组件层。
- **macOS 优先**：先完成 macOS 的桌面体验，再评估后续平台适配。

## 当前状态

已实现 macOS 原生工作台及首个真实 Agent 接入：Codex。使用 Rust、GPUI + GPUI Kit。

- 左侧导航与项目切换，中间工作区；按当前设计暂不显示右侧任务栏。
- 多行输入、项目独立草稿、快捷动作、`⌘K` 项目搜索。
- 首页输入通过 Jev 判断复用已有项目或新建项目，再交给 Codex；判断不明确或接口不可用时手动选择。项目内追问保持当前归属。
- **设置 → 语言** 支持简体中文 / English，首次默认中文，切换立即生效并在重启后保留；不影响草稿和 Agent 对话。
- 内置 Codex 入口，自动发现本地已有安装或选择可执行文件；登录与模型配置沿用本地 Client。Relay 只准备 ACP 适配器，不安装或升级 Harness 本体。
- 输入框中的 **Harness + Model** 菜单，模型与其他选项来自 ACP 实际返回。
- 真实流式对话、工具状态、权限确认、取消、原生会话恢复。
- 按项目保存可见对话和工作目录，重启后恢复；Agent 会话与项目记录独立。
- 真实项目列表：新建、编辑名称/说明/指令；文字资料可添加、编辑、选用与移除，全部独立于 Agent 保存。
- 独立 **文件 / Files** Tab：统一的个人文件空间，导入文件、新建文件和文件夹；编辑文本及 Markdown、预览图片和 PDF / Office 首页。各会话的 Agent 默认将成果保存到同一空间并自动展示，对话里的本地文件链接可直接跳转；保存前检测外部修改，冲突时保留编辑并支持保存副本。
- 本地会话中心：启动同步及每 30 分钟增量读取 Codex 原生会话，覆盖 Relay 之外的对话；支持搜索和项目归属。
- 薄版项目记忆存储与上下文同步；会话 UI 已移除手动记忆管理及回复转存入口，Agent 写入待接入。
- 首轮追加项目上下文快照，后续只追加变化；原生恢复沿用已确认的同步进度，异常后保守重建快照。
- 回复详情显示首字延迟、整轮耗时、上下文交付与可选缓存用量；Codex 用量按最近一次模型请求展示。
- 开发模式默认开启：右键 **检查元素**，或 `⌘⌥I` 打开 GPUI Inspector，再点击目标元素查看布局和样式。
- 原创 Relay 标志及 macOS 应用图标。
- **⌃⌥Space** 全局快捷面板：读取选区与可取得的网页 URL，提供 AI 搜索、解释、翻译、摘要、加入项目；小窗与工作台共享项目主对话。
- **设置 → 语音唤醒**：开启后说「嘿 relay」或「Hey Relay」进入独立语音会话，持续接收后续讲话，点击「结束」或按 Esc 后回到等待唤醒。浮层显示真实麦克风音量；使用本地 sherpa-onnx 中英双语模型，首次需要麦克风权限。
- 关闭窗口后保持常驻；Dock 可重开工作台，Relay 菜单提供快捷面板和退出。
- Relay 使用固定版本 Moli 后台补充网页上下文；抓取未完成或失败不阻塞发送，迟到正文不追加到已发送轮次。

输入草稿只保留在当前窗口中。文字资料可直接粘贴保存；文件和图片可在独立文件 Tab 导入。任务线程、其他 Harness 会话导入、原生会话续聊、对话中的多模态附件、主 Agent 委派、自动划词浮现、视觉尚未实现；语音轮次已接入千问 ASR/TTS，需要语音密钥及 Jev 配置。快捷键选区和网页预取已有基础实现，浏览器兼容性仍需实际权限环境验收。首次升级保留旧项目及已有对话，不迁入示例资料。

点击左侧 **项目 +** 创建项目，标题右上方齿轮编辑项目指令；输入框 **添加上下文** 管理文字资料。只有选用的资料会随下一条消息发送，修改不会影响正在执行的轮次。每份资料最多 12,000 字，选用上下文合计最多 32,000 字，超限时明确提示。

项目记忆由主 Agent 维护，Relay 负责存储与上下文交付；Agent 写入和自动整理仍待接入。会话 UI 不提供记忆添加、编辑、移除或回复转存操作。已有记忆继续随下一条消息进入 ACP 上下文快照或增量，新会话也会获得当前记忆。每个项目最多 32 条记忆，每条正文最多 4,000 字，与项目指令、选用资料共用 32,000 字总额。

项目目录文件 `projects.json` 兼容 v1/v2，保存本地会话来源的记忆时升级为 v3；已有会话检查点继续可读。旧版应用拒绝未知版本，避免丢失记忆来源。

**设置 → 项目自动归属** 支持 OpenRouter、Vercel AI Gateway 和 TypeSafe 三个渠道，默认选择 OpenRouter。可在工作目录 `.env` 中配置 `OPENROUTER_API_KEY`，或在设置中保存对应渠道的 API key。环境配置优先，设置中保存的密钥使用 macOS 钥匙串。请求包含本次输入、项目名称/说明片段、少量近期用户提问，以及每个项目最近新增的最多三条记忆标题（各最多 80 字）；记忆正文和来源不进入 Jev 请求。缺少密钥或额度不足时仍能手动选择项目。接口与完整流程详见 [Jev 项目归属方案](docs/PROJECT-ROUTING.md)。

打开项目输入框的 **Codex** 菜单，选择 **连接本地 Codex**（英文界面为 **Connect local Codex**）。未发现本地安装时先安装 Codex 或在智能体页面选择可执行文件。已有 Codex 登录通常可直接复用；否则按返回的登录方式完成认证。连接后选择模型并发送消息。**智能体 / Agents** 页面管理安装来源与项目工作目录。详见 [Codex 接入](docs/CODEX.md)。

界面语言独立保存在 `~/Library/Application Support/Relay/settings.json`，也遵循 `RELAY_DATA_DIR`。导航、设置、快捷动作、连接状态与已知 ACP 选项随语言切换；用户对话、模型名称、路径与无法识别的上游诊断保留原文。

语音唤醒默认关闭，开关与语言共同保存到应用偏好。首次开启时允许 macOS 麦克风访问；拒绝后可从设置打开系统麦克风权限并重试。Relay 运行且 Mac 保持唤醒时持续监听，关闭工作台仍可使用。设置也提供「开始语音会话」手动入口。唤醒后由本地 Silero VAD 识别人声，连续约 0.8 秒停顿结束一轮，再依次进行 ASR → Prompt → Jev → 项目 Agent 会话 → 千问流式语音播报。处理和朗读期间暂停收音，结束后自动继续听，无需重复唤醒词；可取消本轮重新说话。Jev 归属不明确时选择项目，Agent 授权仍在项目会话确认。单次超过 60 秒会提示重新说话，不拆成多个任务。结束会话恢复关键词检测，关闭开关释放麦克风。音频仅在内存中处理，不录音存档。ASR 使用 `qwen-audio-3.1-asr-flash-message`，TTS 使用 `qwen-audio-3.1-tts-flash`；音频只在唤醒后的完整语句中上传，唤醒词和停顿检测保持本地。未配置语音密钥时明确提示并释放音频。Jev 仍使用独立的 OpenRouter 配置。

后台监听由 Relay 进程中的采音线程负责：切到其他应用、隐藏 Relay、关闭全部窗口后都继续等待唤醒，检测到关键词后重新显示语音浮层。关闭语音浮层仅结束当前会话，恢复等待唤醒；关闭设置中的语音开关或通过 `Cmd+Q` 退出 Relay 才会停止监听。监听期间向 macOS 声明用户发起的持续活动，避免 App Nap 延后处理；该声明随采音线程退出释放，允许 Mac 正常睡眠。不安装独立 daemon 或登录启动项。

语音密钥可通过已签名应用 `dist/Relay.app/Contents/MacOS/relay --voice-save-key` 从标准输入保存到 macOS 钥匙串（输入时请关闭终端回显，例如 `read -s` 后管道传入，勿放在命令参数中）。重启 Relay 后生效，也支持 `DASHSCOPE_API_KEY`，配置项见 `.env.example`。 Jev 的 OpenRouter 密钥可在设置中保存，也可用 `relay --jev-save-key` 从标准输入保存到独立钥匙串条目。默认使用千问统一接口，无需地域或工作空间；阿里云地域密钥可设置 `RELAY_VOICE_REGION` 与 `DASHSCOPE_WORKSPACE_ID`。`relay --voice-check <16k-mono-pcm16.wav>` 是显式云服务检查：上传指定 WAV，输出转写，再播放固定短句；不录制麦克风、不调用 Agent，可能产生少量模型费用。

## 开发与运行

选中文字后按 **Control + Option + Space** 打开小窗。首次读取选区需要在 macOS「隐私与安全性 → 辅助功能」允许 Relay；未授权或应用未暴露选区时可直接粘贴。目标项目和 Codex 连接在小窗中选择。Moli 1.1.10 首次有网页 URL 时后台按需安装，支持用 `RELAY_MOLI_PATH` 指定本地可执行文件。详见 [快捷入口方案](docs/QUICK-ENTRY.md)。

环境：macOS 15+、完整 Xcode（含 Metal 编译器）、Rust 1.97.1。Rust 版本由 `rust-toolchain.toml` 固定，依赖由 `Cargo.lock` 固定。

开发时先复制 `.env.example` 为 `.env`，填写自己的 OpenRouter API key：

```sh
cp .env.example .env
```

```dotenv
OPENROUTER_API_KEY=你的_OpenRouter_API_key
```

Relay 启动时自动读取当前工作目录的 `.env`，该文件已被 Git 忽略。配置优先级为进程环境变量 → `.env` → 钥匙串；环境密钥不写入钥匙串或项目文件，也不会由 `.env` 加载器注入 Agent 子进程环境。修改环境配置后重启 Relay。`cargo xtask start` 通过 macOS Launch Services 启动，并将工作目录设为仓库根目录；通过 Finder 启动时可直接在 **设置 → 项目自动归属 → OpenRouter** 保存密钥。

```sh
cargo xtask verify
cargo xtask bundle
cargo xtask start
```

`verify` 和 `bundle` 使用校验 SHA-256 的固定 sherpa-onnx 1.13.8 静态库；同时校验约 5.2 MiB 的唤醒模型、约 629 KiB 的 Silero VAD 模型及关键词，`bundle` 将这些资源和许可证复制到应用包。首次准备需要下载上游公开归档，之后复用校验后的 target 缓存，不需部署独立服务。直接 Cargo 调试前执行 `cargo xtask prepare-voice`，将打印的库目录设置为 `SHERPA_ONNX_LIB_DIR`；模型自动从 target 下的 `relay-resources/voice` 读取，`RELAY_WAKE_MODEL_DIR` 可显式覆盖，但仍要求清单与校验值一致。包边界和升级约束见 [包治理](docs/PACKAGES.md#语音唤醒领域)。

`start` 复用已运行的 Relay，更新构建后先退出旧进程再启动；启动日志写入 Cargo target 目录的 `relay-launch.log`。选区验收使用这条启动路径，让 macOS 将辅助功能权限归属于 Relay 自身。`cargo run --locked` 仍可用于代码调试，但直接从终端或其他应用启动二进制可能继承父进程的权限归属。

`bundle` 签名优先级为 `RELAY_SIGNING_IDENTITY` → 仓库根目录的 `.relay-signing-identity` → 已有 `dist/Relay.app` 的签名身份；均未配置时才使用 ad-hoc，重编译可能导致系统隐私授权失效，包括辅助功能、麦克风与文稿目录。`.relay-signing-identity` 保存证书名称或 SHA-1，已被 Git 忽略。已有开发者签名会自动沿用，签名失败不会退回 ad-hoc；也可显式设置 `RELAY_SIGNING_IDENTITY=-`。首次从 ad-hoc 切换到稳定证书仍可能需要重新授权。

真实 Jev 联调使用隔离的临时项目，可运行 `cargo run -p relay-runtime --example jev_probe --locked`；添加 `-- --execute --model gpt-6.1-sol` 可进一步验证已安装 Codex 的项目上下文交付。详细结果和客户端兼容性见 [Jev 项目归属方案](docs/PROJECT-ROUTING.md)。

`cargo xtask bundle --release` 生成发布优化的本地应用包。开发检查器由 `devtools` feature 控制；无开发工具的构建可使用 `cargo build -p relay --release --no-default-features --locked`。

## 项目文档

| 文档 | 内容 |
| --- | --- |
| [快捷入口方案](docs/QUICK-ENTRY.md) | 快捷键选区、小窗、Moli 非阻塞网页上下文与首版范围 |
| [产品规格](docs/PRODUCT.md) | 产品定位、项目结构、桌面交互、主 Agent 调度与首版范围 |
| [技术架构](docs/ARCHITECTURE.md) | Rust 模块、ACP 边界、上下文所有权、任务执行与持久化 |
| [UI 框架选型](docs/UI-FRAMEWORK.md) | GPUI + GPUI Kit 的确定方案、比较依据与实现验证 |
| [包治理](docs/PACKAGES.md) | Cargo workspace 分层、依赖约束、自动检查和本地打包 |
| [Codex 接入](docs/CODEX.md) | 本地安装、协议适配器、登录、模型与验证 |
| [个人文件工作区](docs/PERSONAL-FILES.md) | 独立文件 Tab、统一个人目录、编辑与产物预览、保存冲突和静态代码参考 |
| [本地会话中心](docs/CLIENT-SESSIONS.md) | 原生会话导入、30 分钟增量、项目归属与薄记忆 |
| [上下文与缓存落地方案](docs/CONTEXT-CACHING.md) | 项目版本、会话快照、增量同步、异常恢复与用量口径 |
| [Jev 项目归属方案](docs/PROJECT-ROUTING.md) | API、候选项目、判断门槛、自动新建/复用、异常回退与密钥管理 |

设计参考：[Claude Projects redesigned](https://claude.com/blog/projects-redesigned)。Relay 借鉴其项目主对话、执行线程、共享记忆与资料库的组织方式，并连接用户的本地 Agent。

需求基线：2026-09-22。当前依赖 GPUI Kit 0.6.6，底层 GPUI 由 Kit 配套管理。

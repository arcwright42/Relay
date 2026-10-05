# Relay 包治理

应用与开发工具使用 Rust。采用单个 Cargo workspace、单份 Cargo.lock；不为尚未实现的功能创建空 crate。上游 Codex ACP 适配器及所需 Node 属于单独管理的外部运行组件，不作为 Relay 的 UI 或业务实现语言。

## 当前包边界

| 包 | 职责 | 允许的直接依赖 |
| --- | --- | --- |
| `relay` | 应用启动、依赖装配、窗口与退出生命周期 | `relay-ui`、`relay-core`、`relay-runtime`、`relay-platform`、`gpui-kit` |
| `relay-ui` | 工作台、独立语音浮层、thread与文字资料编辑、独立文件 Tab 及预览编辑、对话诊断、中英文文案与原生菜单 | `relay-core`、`gpui-kit` |
| `relay-platform` | macOS 快捷键、选区、浮窗、麦克风权限、本地唤醒检测、VAD 端点与系统朗读 | `relay-core`、`async-channel`、`block2`、`objc2`、`objc2-app-kit`、`objc2-foundation`、`objc2-av-foundation`、`cpal`、`sherpa-onnx` |
| `relay-core` | thread、资料、事实/决策及来源、对话诊断、AgentService / ThreadService / FileService / SettingsService / VoiceService 与采音、转写端口 | 无 |
| `relay-runtime` | 安装、thread与个人文件存储、上下文增量、会话恢复、对话诊断、偏好、SQLite 记忆、任务队列与语音对话轮次 | `relay-core`、`relay-acp`、`rusqlite`、`anyhow`、`dotenvy`、`serde`、`serde_json`、`sha2`、`ureq`、`tokio`、`tokio-tungstenite`、`futures-util`、`uuid`、`security-framework`（macOS） |
| `relay-acp` | ACP v1 协商、Agent 进程、认证、模型配置、流式事件、权限和取消 | `relay-core`、`agent-client-protocol`、`async-channel`、`async-io`、`futures-lite`、`serde_json` |
| `xtask` | 包边界、质量检查、固定资源校验、图标、本地 macOS 打包与启动 | `serde_json`、`sha2` |

`relay-ui` 内按工作台状态、导航、对话输入、辅助页面和开发工具组织模块。`devtools` feature 默认从应用入口开启，统一启用 GPUI 检查器与 Relay 右键入口；关闭默认 feature 可以剔除这些开发入口。

运行依赖方向是 `relay → relay-ui → relay-core` 和 `relay → relay-runtime → relay-acp → relay-core`。入口注入 AgentService、ThreadService 和 SettingsService；UI 只使用领域命令和快照，不依赖 ACP 或进程 API。ACP SDK 类型不会穿透到 UI 或领域包。`relay-core` 不依赖 UI、ACP SDK、异步运行时或平台 API。thread预览资料已移除。

当前thread存储放在 `relay-runtime::resident`，快照和增量在 `context`，对话与检查点在 `store`，诊断序列化在 `metrics`。ThreadService 的 apply 在 UI 后台 executor 调用，只有原子保存完成才发布新版本；快照读取不做磁盘 I/O。macOS 能力已拆入 `relay-platform`；SQLite 已在 runtime 内实现，不新增数据库服务或 workspace crate。

个人文件工作区复用现有包：`relay-core::files` 定义文件列表、内容、位置、命令与 `FileService`，`relay-runtime::files` 执行后台目录读取、导入和冲突检查保存，`relay-ui::workbench::files` 提供独立文件 Tab。应用入口装配 FileStore，使用应用级统一个人目录，不依赖 ThreadService 或 AgentService。各会话的 Agent 默认使用同一目录。PDF / Office 首页预览由 macOS Quick Look 在后台生成，不新增直接依赖或数据库。

薄版thread记忆复用这些边界：领域定义 MemoryItem，projects 保存事实/决策及回复来源，context 生成记忆快照和增量，routing 只读取少量标题。记忆由主 Agent 维护，UI 已移除手动记忆管理及回复转存入口，Agent 写入待接入。没有新增依赖、workspace crate、数据库或外部服务。


ACP SDK 仍固定 2.2.0，仅显式开启 `unstable_end_turn_token_usage` 来读取可选用量；不启用整个 unstable 集合，不升级 lockfile。缺失或损坏的可选 usage 不影响对话。测试专用 `relay-runtime/test-support` 只转发 `relay-acp/test-support`，用内存传输验证快照交付、写入顺序与失败恢复；产品默认构建不含测试传输。

应用偏好通过独立的 `SettingsService` 注入 UI，由 `relay-runtime::settings` 保存到应用级 `settings.json`。切换语言先更新内存，单个后台写入线程合并并按顺序保存，使用临时文件、sync 与原子替换；损坏或不支持的文件保留原样并显示错误。UI 的编译期文案表要求每项同时有中英文，不新增依赖。语言切换同步组件及原生菜单，不发送 Agent 命令，不改thread对话、草稿或 ACP 配置 ID。

`relay-platform` 是 macOS FFI 的独立边界：Carbon 注册 Control + Option + Space，事件通过容量为 1 的异步通道送给 UI；辅助功能在后台读取选区及窗口 AXDocument。该包因系统 FFI 显式使用 unsafe，并要求 unsafe_op_in_unsafe_fn = deny；其余包继续 forbid unsafe。注册句柄只在主线程创建和释放，CF 对象按所有权释放。UI 不直接依赖平台包。

`relay-core::capture` 定义不可变选区、动作和网页抓取接口；`relay-runtime::webfetch` 后台执行 Moli，UI 在点击发送时只组装当时已取得的材料。

### 语音唤醒领域

`relay-core::voice` 定义会话/轮次快照、错误和采音、资源、ASR、thread对话、朗读端口（`VoiceInputBackend`、`WakeResources`、`SpeechTranscriber`、`VoicePromptService`、`SpeechOutput`、`SpeechAudioOutput`、`SpeechAudioStream`）。快照不包含模型句柄、API key 或原始 PCM。`relay-runtime::voice` 分为生命周期 `mod`、资源 `resources`、轮次编排 `turns`、thread执行 `dialogue`、云语音 `cloud/{config,transport,asr,tts}`。偏好持久化、generation、会话和轮次编号隔离旧结果；唤醒冷却为 3 秒。

`relay-platform::voice` 按 `permission / activity / audio / detector / speech / playback / output` 管理 macOS 权限、后台活动声明、CPAL 采音、sherpa-onnx KWS、Silero VAD、流式 PCM 播放与备用系统语音。采音线程持有 `activity::ListeningActivity`，通过 `NSProcessInfo` 防止监听期间进入 App Nap，同时允许系统空闲睡眠；关闭监听、采音失败和应用退出均通过 RAII 释放，不依赖窗口生命周期，不创建独立服务。只有平台包依赖原生 SDK，音频回调使用有界队列，不等待推理；丢帧、输入设备变化、处理或朗读期间均重置缓冲。KWS 流定期重建并保留短重叠。会话中以 16 kHz、512 样本窗口运行 VAD；默认阈值 0.5，最少人声 0.25 秒、连续静音约 0.8 秒后提交完整语句。静音和低幅背景噪声不提交；超出 60 秒丢弃整句并等待手动继续，不以截断内容执行任务。领域载荷为单声道 16 kHz PCM16 WAV，音量事件携带会话编号。

`turns` 串行执行 ASR → 主对话 Agent → TTS，处理期间暂停新语句，队列最多 1 个待处理任务，防止取消切换造成无限积压。唤醒后第一句的唤醒前缀会被移除；空转写不发送。所有异步进度检查 generation、session ID、turn ID 和取消标记。最近转写和可见回复各限制 32,000 字符。`UnconfiguredTranscriber` 明确报告待配置，音频立即释放，不积压、不上传。实际入口注入 `QwenSpeech`；没有密钥时不上传，配置失败或远端失败会显示错误，不能把部分转写当完整任务。

`ThreadVoiceDialogue` 复用 `ThreadService / AgentService`，直接进入逻辑主对话 0。已有thread复用 Agent 会话和上下文机制，不覆盖文本草稿，不向忙碌的 Agent 插入任务，不自动登录或批准工具授权。`AgentService::send_turn` 在派发时返回本轮 assistant message ID，回复、失败和取消均绑定该 ID；`CancelTurn` 只取消仍是当前轮次的请求，不能取消后来从其他窗口发送的任务。

`QwenSpeech` 同时实现 ASR 和 TTS 端口。固定 ASR `qwen-audio-3.1-asr-flash-message` 与 TTS `qwen-audio-3.1-tts-flash`，中文音色 `longanhuan_v3.1`、英文 `Annie_v3.1`。默认连接千问官方 `maas.qianwenaiapi.com`，保留北京/新加坡及工作空间域名配置。密钥优先读取进程环境、工作目录 `.env`、独立 macOS Keychain 条目；不导出到进程环境，不与 Harness 凭据混用，不记录请求或服务端原始错误。配置只允许官方域名，不自动尝试其他服务。

`transport` 使用固定 `tokio 1.53.1`、`tokio-tungstenite 0.30.0`（Rustls/WebPKI）、`futures-util 0.3.34` 和 `uuid 1.26.1`。每个任务拥有独立 WebSocket 和 UUID，先等待 task-started；ASR 上传经过格式/长度检查的 16 kHz PCM，只接收终态句子并按 sentence ID 去重，task-finished 后才交给主对话。握手 10 秒、任务启动 15 秒、ASR 总计 90 秒、TTS 总计 330 秒超时；取消覆盖连接、上传、等待结果和播放。单帧/消息最大 512 KiB，转写最多 32 KiB，合成音频最多 5 分钟。自动化测试使用本机 WebSocket 服务和假密钥，不调用云服务。

TTS 使用 24 kHz PCM16 流，收到音频就交给 `MacAudioOutput`，平台层用 CPAL 播放和重采样，最多缓存 4 秒并反压，不把音频写入磁盘。完成或取消时销毁输出流；播放完成包含硬件缓冲时间，然后轮次层再等待 300 ms 尾音才恢复收音。共同的 `spoken_text` 策略跳过代码块、最多朗读 3,000 字。保留 `MacSpeechOutput` 作为可显式注入的系统声音实现，云端失败不会静默切换。默认半双工，用户可点「取消并重新说话」；当前不支持用人声打断播放。

入口在创建 ResidentStore、AgentRuntime 和 ResidentWorker 后组装语音管线并管理独立 `VoicePanel`。UI 只显示阶段、当前请求/回复/任务、继续和结束按钮，不处理音频或直接发送 Agent 命令。打开thread会话复用现有窗口事件；关闭浮层、点击结束或 Esc 结束会话并恢复等待唤醒，旧窗口不能结束新会话。设置默认关闭，开关和语言由同一偏好写入线程合并持久化。

构建固定 `sherpa-onnx 1.13.8`（macOS static）、`cpal 0.16.0`、`objc2-av-foundation 0.3.2`。`crates/relay-runtime/resources/voice/manifest.json` 固定引擎、模型、VAD、关键词和许可证的来源及 SHA-256：中英双语 3M Zipformer，chunk-16，int8 encoder/joiner、fp32 decoder，约 5.2 MiB；Silero VAD 643,854 字节。`verify / bundle` 准备并校验资源，运行时重新验证。模型和库缓存于 Cargo target，随应用资源交付，不进入 Git、不部署独立服务。平台测试使用仓库内的短合成 WAV 验证实际 VAD，runtime 使用受控端口验证整条轮次与取消，测试不调用远程 ASR 或真实 Agent。

## 依赖和版本

- 直接依赖只在根 `Cargo.toml` 的 `workspace.dependencies` 中声明，成员包使用 `workspace = true` 继承。
- 当前使用 Rust 1.97.1、GPUI Kit 0.6.6；保留 `rust-toolchain.toml` 和 `Cargo.lock`。直接依赖固定精确版本，传递依赖由 lockfile 固定。
- GPUI Kit 负责匹配 GPUI 版本。成员包不单独引入另一个 GPUI 版本，也不从 Git 分支拉取框架。
- 版本、edition、最低 Rust 版本、发布策略与 lint 从 workspace 继承。所有内部包均为 `publish = false`。
- 升级依赖使用单独的变更，说明原因、API 变化和验证结果；GPUI Kit 与它的 GPUI 依赖作为一个兼容组合验证。
- 不运行无范围的 `cargo update` 来解决编译问题。不用通配版本或未固定的 Git 依赖。
- 检查重复传递依赖使用 `cargo tree --workspace --duplicates`；由上游引入的重复版本先定位原因，再决定是否调整。

当前有一个受控的上游补丁：`gpui-pre 0.3.6` 的 Inspector 在选取元素时会拦截自身关闭按钮的点击。根清单通过 `[patch.crates-io]` 指向仓库内源码，只修正点击分发范围，保留版本和许可证。来源、原始包校验值、补丁内容与移除条件见 [vendor/README.md](../vendor/README.md)。禁止通过修改本机 Cargo 缓存修复依赖，也不把 vendor 包作为 Relay 的 workspace 成员。

## 自动检查

```sh
cargo xtask check-packages
cargo xtask verify
```

`check-packages` 读取 Cargo 的解析结果，检查允许依赖方向、精确版本、禁止 Git 分支、统一包版本和禁止发布；同时检查适配器 npm 清单与 lockfile 的一致性，禁止直接依赖 Codex CLI 或使用 override 固定它，每个下载包必须固定版本、来自指定 registry 并带 SHA-512 integrity。唤醒检查同时约束原生资源清单、SDK 版本、macOS 目标和 static feature。规则包含反向依赖、浮动版本、缺失校验值及唤醒依赖漂移的回归测试。

`verify` 依次执行包边界检查、格式检查、workspace 全目标全 feature Clippy（警告作为错误）与全 feature 测试。`relay-acp/test-support` 启用测试 ACP 子进程及内存命令传输，覆盖登录、模型、权限、取消、恢复去重、用量和进程退出；不会打进应用包。UI 的开发依赖额外启用 GPUI Kit 的 `test-support`，用真实鼠标/键盘事件覆盖thread与资料表单、资料选择，以及 Inspector 关闭与普通点击恢复。测试不需要网络、真实账号或模型费用。GitHub Actions 的 macOS 工作流运行同一条命令；CI 依赖的 Actions 固定到提交 SHA。

## 外部运行组件

当前兼容组合为 Rust ACP SDK 2.2.0、ACP 协议 v1、codex-acp 1.12.0、用户本地 Codex、Node 24.21.0。应用内嵌协议适配器清单和 npm lockfile，按需下载到 Relay 的 Application Support 目录。Node 的 Apple Silicon / Intel 官方归档分别固定 SHA-256；npm 使用 `ci --ignore-scripts --omit=optional` 和仓库内 lockfile，不运行 `npx ...@latest`，不使用用户的全局 npm 安装位置。

组件先安装到临时目录，校验版本后再激活；失败保留当前版本，替换失败恢复原目录。旧版本目录保留，尚无用户可操作的升级/回滚界面。选择本地 Codex 时，只有 Codex 可执行文件来自用户指定路径，适配器和 Node 仍由 Relay 管理。升级须同步清单、lockfile、Rust 中的版本标识、校验值及真实连接验证记录。详见 [Codex 接入](CODEX.md)。

## 本地交付

Moli 是独立的按需网页采集组件，固定 1.1.10。`moli_installer` 使用官方 macOS Apple Silicon / Intel 归档的固定 SHA-256，复用校验、临时目录与原子激活流程；不执行在线安装脚本。`webfetch` 将 stdout 写入仅当前用户可读的临时文件，限制正文与总输出大小，超时/退出结束进程组并清理。许可证随原发布归档保留；`RELAY_MOLI_PATH` 是显式本地覆盖入口。

```sh
cargo xtask icon
cargo xtask bundle
cargo xtask start
```

图标以 `assets/relay-icon.svg` 为应用图标源文件，`assets/relay-mark.svg` 用于单色界面标志。`icon` 使用 macOS 系统工具生成 PNG 和 ICNS；调整矢量稿后重新生成并提交这些资源。

`bundle` 默认使用开发构建，`bundle --release` 使用发布优化，均生成 `dist/Relay.app`。在覆盖可执行文件之前解析签名身份：优先使用 `RELAY_SIGNING_IDENTITY`，其次使用被 Git 忽略的 `.relay-signing-identity`，再沿用已有包的签名证书；新环境未配置证书时才采用 ad-hoc 并发出授权失效提示。身份配置为空、旧签名无法识别或签名失败时直接报错，不静默降级；签名后执行 `codesign --verify --strict`。

可执行文件通过临时文件和 rename 替换，避免覆盖正在运行进程所映射的文件；已打开的窗口继续使用旧构建，新构建下次启动生效。`start` 通过 Launch Services 打开应用，复用已有实例，不终止正在进行的对话。仅传递非密钥的 `RELAY_WORKING_DIRECTORY` 以恢复仓库工作目录和 `.env` 读取；stdout/stderr 写到 Cargo target 下的 `relay-launch.log`。直接执行二进制的 `cargo run --locked` 适合代码调试，原生辅助功能验收应使用 `start`。

对外分发所需的 notarization 和更新机制尚未接入。构建产物、编辑器配置、本机签名身份与日志不进入 Git。

本地 Client 会话归档属于 relay-runtime，后台归档使用 relay-core::sessions 的快照和只读打开/同步命令，记忆页面通过 relay-core::memory 读取 Claude-Mem 观察与投递状态。首个 Codex 解析器只读取原生 JSONL；磁盘归档、增量游标与 30 分钟调度不依赖 ACP 会话创建或发送。见 [归档与记忆入口](CLIENT-SESSIONS.md)。

## 记忆 Provider 和任务

`relay-runtime::memory` 定义 Memory Provider 和统一 stdio MCP 入口，当前只适配 Claude-Mem；应用、上下文、事件采集、后台工作和 UI 选择同一个 Provider。MCP 的任务工具仍属于 Relay，记忆工具由 Provider 提供。入口为 `relay --relay-mcp <data-dir> <thread-id> [provider-fingerprint]`，stdio 只输出 JSON-RPC。

`relay-runtime::resident` 使用固定 `rusqlite 0.40.2`（bundled SQLite），只拥有任务、会话、消息编号与兼容迁移。`ResidentStore` 是 ThreadService；`ResidentWorker` 负责任务请求、结果通知和显式启用的历史扫描。Native 记忆源码、Observer、Embedding、FTS/向量引擎和专属 CLI 已删除。Claude-Mem adapter 通过现有 ureq 接入本地 Worker，Relay 的 memory SQLite 仅用于投递日志；未新增 Rust 依赖。旧 JSON ThreadStore 仅供兼容和测试。详见 [Memory Provider](MEMORY-PROVIDERS.md)。

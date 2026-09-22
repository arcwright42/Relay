# Relay 包治理

应用与开发工具使用 Rust。采用单个 Cargo workspace、单份 Cargo.lock；不为尚未实现的功能创建空 crate。上游 Codex ACP 适配器及所需 Node 属于单独管理的外部运行组件，不作为 Relay 的 UI 或业务实现语言。

## 当前包边界

| 包 | 职责 | 允许的直接依赖 |
| --- | --- | --- |
| `relay` | 应用启动、依赖装配、窗口与退出生命周期 | `relay-ui`、`relay-core`、`relay-runtime`、`gpui-kit` |
| `relay-ui` | 工作台、项目与文字资料编辑、对话诊断、中英文文案与原生菜单 | `relay-core`、`gpui-kit` |
| `relay-core` | 项目、资料、对话诊断、AgentService / ProjectService / SettingsService 接口 | 无 |
| `relay-runtime` | 安装、项目存储、确定性上下文快照与增量、会话恢复、对话诊断与应用偏好持久化 | `relay-core`、`relay-acp`、`anyhow`、`serde`、`serde_json`、`sha2` |
| `relay-acp` | ACP v1 协商、Agent 进程、认证、模型配置、流式事件、权限和取消 | `relay-core`、`agent-client-protocol`、`async-channel`、`async-io`、`futures-lite`、`serde_json` |
| `xtask` | 包边界检查、质量检查、图标生成与本地 macOS 打包 | `serde_json` |

`relay-ui` 内按工作台状态、导航、对话输入、辅助页面和开发工具组织模块。`devtools` feature 默认从应用入口开启，统一启用 GPUI 检查器与 Relay 右键入口；关闭默认 feature 可以剔除这些开发入口。

运行依赖方向是 `relay → relay-ui → relay-core` 和 `relay → relay-runtime → relay-acp → relay-core`。入口注入 AgentService、ProjectService 和 SettingsService；UI 只使用领域命令和快照，不依赖 ACP 或进程 API。ACP SDK 类型不会穿透到 UI 或领域包。`relay-core` 不依赖 UI、ACP SDK、异步运行时或平台 API。项目预览资料已移除。

当前项目存储放在 `relay-runtime::projects`，快照和增量在 `context`，对话与检查点在 `store`，诊断序列化在 `metrics`。ProjectService 的 apply 在 UI 后台 executor 调用，只有原子保存完成才发布新版本；快照读取不做磁盘 I/O。继续沿用现有 crate，未来数据库和 macOS 能力需要独立边界时再拆分；接入前同步允许依赖图。

ACP SDK 仍固定 2.2.0，仅显式开启 `unstable_end_turn_token_usage` 来读取可选用量；不启用整个 unstable 集合，不升级 lockfile。缺失或损坏的可选 usage 不影响对话。测试专用 `relay-runtime/test-support` 只转发 `relay-acp/test-support`，用内存传输验证快照交付、写入顺序与失败恢复；产品默认构建不含测试传输。

应用偏好通过独立的 `SettingsService` 注入 UI，由 `relay-runtime::settings` 保存到应用级 `settings.json`。切换语言先更新内存，单个后台写入线程合并并按顺序保存，使用临时文件、sync 与原子替换；损坏或不支持的文件保留原样并显示错误。UI 的编译期文案表要求每项同时有中英文，不新增依赖。语言切换同步组件及原生菜单，不发送 Agent 命令，不改项目对话、草稿或 ACP 配置 ID。

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

`check-packages` 读取 Cargo 的解析结果，检查允许依赖方向、精确版本、禁止 Git 分支、统一包版本和禁止发布；同时检查托管 npm 清单、override、lockfile 的一致性，每个下载包必须固定版本、来自指定 registry 并带 SHA-512 integrity。规则包含反向依赖、浮动版本和缺失校验值的回归测试。

`verify` 依次执行包边界检查、格式检查、workspace 全目标全 feature Clippy（警告作为错误）与全 feature 测试。`relay-acp/test-support` 启用测试 ACP 子进程及内存命令传输，覆盖登录、模型、权限、取消、恢复去重、用量和进程退出；不会打进应用包。UI 的开发依赖额外启用 GPUI Kit 的 `test-support`，用真实鼠标/键盘事件覆盖项目与资料表单、资料选择，以及 Inspector 关闭与普通点击恢复。测试不需要网络、真实账号或模型费用。GitHub Actions 的 macOS 工作流运行同一条命令；CI 依赖的 Actions 固定到提交 SHA。

## 外部运行组件

当前兼容组合为 Rust ACP SDK 2.2.0、ACP 协议 v1、codex-acp 1.12.0、Codex 0.154.0、Node 24.21.0。应用内嵌托管安装清单和 npm lockfile，按需下载到 Relay 的 Application Support 目录。Node 的 Apple Silicon / Intel 官方归档分别固定 SHA-256；npm 使用 `ci --ignore-scripts` 和仓库内 lockfile，不运行 `npx ...@latest`，不使用用户的全局 npm 安装位置。

组件先安装到临时目录，校验版本后再激活；失败保留当前版本，替换失败恢复原目录。旧版本目录保留，尚无用户可操作的升级/回滚界面。选择本地 Codex 时，只有 Codex 可执行文件来自用户指定路径，适配器和 Node 仍由 Relay 管理。升级须同步清单、lockfile、Rust 中的版本标识、校验值及真实连接验证记录。详见 [Codex 接入](CODEX.md)。

## 本地交付

```sh
cargo run --locked
cargo xtask icon
cargo xtask bundle
open dist/Relay.app
```

图标以 `assets/relay-icon.svg` 为应用图标源文件，`assets/relay-mark.svg` 用于单色界面标志。`icon` 使用 macOS 系统工具生成 PNG 和 ICNS；调整矢量稿后重新生成并提交这些资源。

`bundle` 默认使用开发构建，`bundle --release` 使用发布优化，均生成 `dist/Relay.app` 并做本地 ad hoc 签名。可执行文件通过临时文件和 rename 替换，避免覆盖正在运行进程所映射的文件；已打开的窗口继续使用旧构建，新构建下次启动生效。对外分发所需的开发者签名、notarization 和更新机制尚未接入。构建产物、编辑器配置与日志不进入 Git。

# Relay 包治理

应用与开发工具都使用 Rust。采用单个 Cargo workspace、单份 Cargo.lock；不为尚未实现的功能创建空 crate。

## 当前包边界

| 包 | 职责 | 允许的直接依赖 |
| --- | --- | --- |
| `relay` | 应用启动、窗口、原生菜单与退出生命周期 | `relay-ui`、`gpui-kit` |
| `relay-ui` | 工作台、输入状态、导航、视觉资源引用与示例数据 | `relay-core`、`gpui-kit` |
| `relay-core` | 项目标识、上下文类型和领域数据 | 无 |
| `xtask` | 包边界检查、质量检查、图标生成与本地 macOS 打包 | `serde_json` |

`relay-ui` 内按工作台状态、导航、对话输入、辅助页面和开发工具组织模块。`devtools` feature 默认从应用入口开启，统一启用 GPUI 检查器与 Relay 右键入口；关闭默认 feature 可以剔除这些开发入口。

运行依赖方向是 `relay → relay-ui → relay-core`。`relay-core` 不依赖 UI、ACP SDK、异步运行时或平台 API。示例任务与示例资料只放在 `relay-ui::preview`，不进入核心领域包。

新功能在有具体实现时再引入相应包：`relay-acp` 管理协议与 Agent 进程，`relay-storage` 实现项目持久化，`relay-platform` 封装 macOS 能力。它们可以依赖核心类型，核心类型不能反向依赖它们。接入前同时更新此文档与 `xtask` 的允许依赖图。

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

`check-packages` 读取 Cargo 的解析结果，检查允许依赖方向、精确版本、禁止 Git 分支、统一包版本和禁止发布。规则包含反向依赖和浮动依赖的回归测试。

`verify` 依次执行包边界检查、格式检查、workspace 全目标 Clippy（警告作为错误）与 workspace 测试。UI 的开发依赖只额外启用 GPUI Kit 的 `test-support`，用真实鼠标事件覆盖 Inspector 选取中关闭、选中后关闭、重新打开和恢复普通点击。GitHub Actions 的 macOS 工作流运行同一条命令；CI 依赖的 Actions 固定到提交 SHA。

## 本地交付

```sh
cargo run --locked
cargo xtask icon
cargo xtask bundle
open dist/Relay.app
```

图标以 `assets/relay-icon.svg` 为应用图标源文件，`assets/relay-mark.svg` 用于单色界面标志。`icon` 使用 macOS 系统工具生成 PNG 和 ICNS；调整矢量稿后重新生成并提交这些资源。

`bundle` 默认使用开发构建，`bundle --release` 使用发布优化，均生成 `dist/Relay.app` 并做本地 ad hoc 签名。对外分发所需的开发者签名、notarization 和更新机制尚未接入。构建产物、编辑器配置与日志不进入 Git。

# Codex 接入

当前首个 Harness 为 Codex。应用内置其入口和安装清单，首次连接按需准备运行组件；扫描已有安装是可选操作。Relay 的 UI、状态管理、存储、安装器和 ACP 客户端均为 Rust，上游 `codex-acp` 的 JavaScript 与 Node 作为独立组件管理。

## 使用流程

1. 在项目输入框点击 **Codex**，再点击 **Set up Codex / Connect Codex**。安装过程显示下载、准备、连接状态，可取消并重试。
2. 已有 Codex 登录由 Codex 自身复用。需要认证时选择 **ChatGPT** 打开浏览器；**API Key (from environment)** 使用应用启动环境中的 `CODEX_API_KEY` / `OPENAI_API_KEY`。当前没有自定义供应商或密钥输入界面。Relay 不复制密钥到项目文件。
3. 连接后，菜单显示 **Codex · 当前模型**。模型、推理强度和模式等选项来自当前 Agent 的 `configOptions`，不维护硬编码模型表。切换等待 Agent 确认，失败时保留实际配置并显示原因。
4. 输入消息，点击发送或 `⌘Enter`。回复流式显示；工具需要审批时按 Agent 提供的选项允许或拒绝。停止按钮发送 ACP 取消，收到终态后才允许下一轮；10 秒内未确认则关闭连接并保留未完成标记。
5. **Agents** 页面管理所选项目的安装来源、连接和工作目录。默认每个项目使用独立的 Relay 工作目录；可以改为现有文件夹。改变工作目录会重建执行会话，可见对话仍属于原项目。

工作目录不是系统沙箱边界；工具执行权限由 Codex 的模式、沙箱及审批共同落实。项目指令和选用的文字资料会随消息发送，示例资料已移除。图片、文件附件、语音、划词入口及多 Agent 委派暂未接通。

## 组件与包治理

| 组件 | 固定版本 / 策略 |
| --- | --- |
| Rust `agent-client-protocol` | 2.2.0，协议协商 v1 |
| `@agentclientprotocol/codex-acp` | 1.12.0 |
| `@openai/codex` | 0.154.0，直接依赖和 override 同时固定 |
| Node | 24.21.0，macOS arm64 / x64 官方归档及 SHA-256 |

清单位于 `crates/relay-runtime/resources/codex/`。Node 由 macOS 系统 `curl` 下载、校验后由 `tar` 解压；npm 使用独立缓存和配置文件执行 `ci --ignore-scripts`，lockfile 为每个包锁定 URL、版本和 SHA-512 integrity。无需预装 Node、npm 或 Codex。

安装先进入临时目录，检查版本与 lockfile 后原子激活，失败不覆盖可用版本。旧目录保留供后续治理，尚未提供在线升级和用户回滚菜单。连接时不会运行 `npx @latest` 或修改全局 npm 安装。使用本地 Codex 时检查绝对路径和版本输出，适配器通过 `CODEX_PATH` 调用它；本地版本兼容性由实际握手和会话检查确认。

常见路径扫描只检查 `PATH`、Homebrew、用户 npm/Volta/asdf 与有限数量的 nvm 版本目录，不递归扫描整块磁盘，也不在扫描时执行候选程序。

## 数据与线程边界

默认目录为 `~/Library/Application Support/Relay`；开发测试可通过 `RELAY_DATA_DIR` 覆盖：

```text
Relay/
  projects.json
  components/node/24.21.0/
  components/agents/codex-acp-1.12.0-codex-0.154.0/
  cache/npm/
  projects/<project-id>/conversation.json
  projects/<project-id>/workspace/
```

`projects.json` 保存项目定义、指令和文字资料及其版本，独立于 Agent。`conversation.json` v2 保存可见消息、工具摘要、工作目录、来源、已确认配置、原生 session ID、上下文同步检查点及对话诊断；兼容读取 v1。文件使用 0600 权限和临时文件替换；发送前先写入未确认状态，写入失败不发送 prompt。流式内容约每秒检查点，轮次结束与正常退出时刷新。恢复后将未结束的回复标记为 interrupted。无法解析或版本不支持的文件保留原样，禁止写入覆盖。

每个项目拥有独立 ACP 连接和状态。连接 generation 拒绝已替换进程的迟到事件。安装、协议和存储不在 GPUI 渲染线程执行；退出时停止进程组并刷新记录。UI 仅访问 `relay-core::AgentService` 的命令和修订快照。

再次连接优先执行原生 `session/load`，不重复插入 Agent 重播的历史或已确认项目资料。无法复用时创建新会话，追加当前项目快照，再将 Relay 保存的近期可见对话作为历史上下文，最多 24,000 字符。正常后续轮次仅追加资料变化；取消、拒绝或失败后保守重发当前快照。隐藏推理与 Agent 内部缓存不迁移；输入草稿尚未持久化。详见 [上下文与缓存方案](CONTEXT-CACHING.md)。

每条回复的 **回复详情 / Response details** 显示首个文字块延迟、包含工具和审批等待的整轮耗时、项目上下文交付及可选 token 用量。固定 codex-acp 1.12.0 的 usage 来自最后一次模型请求，并将缓存读取从 inputTokens 中扣除；因此这些计数不代表包含全部工具循环的整轮总量。缺失 usage 或缓存读取计数时显示暂无数据，不伪造 0% 命中率。

## 验证

```sh
cargo xtask verify
cargo run -p relay-runtime --example codex_probe
cargo run -p relay-runtime --example codex_probe -- --prompt 'Reply with only: Relay connected.'
```

`verify` 不依赖真实账号或网络，使用 Rust ACP 子进程验证认证、模型与配置确认、权限等待、取消、恢复去重、进程回收；另有项目隔离、损坏文件保护、退出保存、安装校验及包规则测试。

`codex_probe` 是主动执行的真实连接检查，数据和组件位于数据目录的 `probes/codex/`，不进入用户项目目录。默认仅安装、连接并读取模型数量，不调用推理；`--prompt` 才发送真实消息并产生正常模型用量。会复用现有 Codex 认证；没有登录时退出并提示从应用登录。必要时设置独立 `RELAY_DATA_DIR`。旧版本 probe 的项目 9001 不迁入真实项目列表。

2026-09-22 在 Apple Silicon macOS 完成从空组件目录的托管安装、ACP 握手、5 个模型的动态发现，以及原生 UI 的模型切换、真实回复、项目切换、可选本地扫描和重启恢复验证。新账号的浏览器 OAuth 最终授权需要账号本人操作；协议认证状态机由离线测试覆盖。Intel 归档已配置校验值，尚未在 Intel 硬件上运行验证。

## 设计依据

- [Zed 外部 Agent](https://zed.dev/docs/ai/external-agents)：ACP 连接与 Agent 安装/配置分开。
- [Codeg 支持的 Agent](https://docs.codeg.app/guide/supported-agents)：托管运行组件与已有安装可以共存。
- [Rust ACP SDK](https://github.com/agentclientprotocol/rust-sdk)：Rust 客户端与进程连接。
- [Codex ACP v1.12.0](https://github.com/agentclientprotocol/codex-acp/tree/v1.12.0)：适配器、认证及 `CODEX_PATH`。
- [ACP Session Config Options](https://agentclientprotocol.com/protocol/v1/session-config-options)：模型与其他选项由 Agent 动态声明。

这些产品用于验证接入方式；Relay 的项目存储和未来主 Agent 调度仍由自身领域模型管理。

# Jev 项目归属判断

更新：2026-09-30。用户在首页直接输入需求，由 Jev 判断继续哪个项目或创建新项目，再进入项目，通过 ACP 交给 Codex。项目上下文仍由 Relay 独立保存。项目归属判断发生在执行之前，与项目内主 Agent 的任务拆分、委派分开。

## 使用流程

1. 默认使用 OpenRouter：在当前工作目录的 `.env` 中填写 `OPENROUTER_API_KEY`，或通过启动进程环境变量提供。也可在 **设置 → 项目自动归属** 选择 OpenRouter、Vercel AI Gateway 或 TypeSafe，保存对应渠道的 API key。OpenRouter 密钥来自 [OpenRouter 控制台](https://openrouter.ai/settings/keys)；Vercel 使用 [AI Gateway key](https://vercel.com/docs/ai-gateway/authentication-and-byok/api-keys)，TypeSafe 使用 [直连密钥](https://console.typesafe.ai/keys)。配置状态表示本机已读到密钥，首次请求才验证服务端权限。
2. 在首页输入需求并发送。Jev 从已有项目、新项目、需要用户选择中判断。
3. 证据充足时，Relay 直接进入相应项目；需要新项目时先原子保存，再交给 Agent。Jev 不生成项目名称，新项目名称取原输入前 60 个字符，说明取前 600 个字符，用户可以编辑；原始完整提问仍作为本轮消息保留。
4. 目标 Codex 已就绪时直接发送；尚未连接时自动连接该项目配置的 Codex，必要时由用户登录。界面显示待发送状态；取消、切换页面、修改草稿或连接失败后保留草稿，取消自动发送。目标正在运行任务时只保留草稿，不在旧任务结束后意外发送。
5. 判断不清、没有密钥、超时、限流或错误响应时，保留首页输入，展示项目选择与新建按钮，由用户决定；不把接口失败视为“没有合适项目”。

明确打开一个项目后，输入归属于这个项目，绕过 Jev，避免每次追问又被分配到别处。首页是本次 Jev 接入入口；现有快捷面板使用手动选择项目，自动归属可在后续复用 RoutingService。语音入口尚未实现。

## 官方接口与输入

通过 Rust HTTPS 客户端及 Bearer 认证调用，一个 Choice 问题 `destination`。三个渠道使用同一 TypeSafe 兼容结构：请求包含 `state`、`questions`、`instructions` 和 `criteria`，响应读取 `answers.destination` 的 `choice`、`probabilities` 和 `confidence`，不解析自由文本。

| 渠道 | 固定地址 | 请求和响应模型 ID |
| --- | --- | --- |
| OpenRouter（默认） | `https://openrouter.ai/api/alpha/decisions` | 请求 `typesafe/jev-1.13`，响应允许同名或 `typesafe/jev-1.13-YYYYMMDD` |
| TypeSafe | `https://api.typesafe.ai/v1/systemone` | `jev-1.13.0` |
| Vercel AI Gateway | `https://ai-gateway.vercel.sh/typesafe/v1/systemone` | `typesafe-ai/jev` |

OpenRouter 使用专用 Decisions API，不使用聊天补全接口。[官方响应示例](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-request) 返回 `typesafe/jev-1.13-20260917`；验证允许请求模型名后追加八位数字的版本日期，拒绝其他模型或版本。返回的实际模型 ID 保留在决策中。Vercel 使用网关模型别名，TypeSafe 使用固定版本，两个渠道保持原有的模型 ID 严格匹配。

渠道与钥匙串密钥作为一个记录保存，切换选择不会立即激活，也不会将已有密钥发送到另一家服务；必须保存对应的新密钥。环境配置始终绑定 OpenRouter。错误不自动切换渠道。

2026-09-22 查询：[Vercel Jev 模型页](https://vercel.com/ai-gateway/models/jev) 标注限免至 2026-09-25，且列入 [免费层模型目录](https://vercel.com/ai-gateway/models?freeTier=true)。公开模型 API 的基础标价仍为 $0.042 / 百万输入 token，促销是否适用以账户实际显示为准。Vercel [免费层](https://vercel.com/docs/ai-gateway/pricing) 每月 $5 仅适用于合资格模型，并有调用频率限制；购买网关额度后不再享受每月赠额。应用不内置永久免费的承诺，不代为充值或开启自动充值；额度不足时保留输入并提供手动选择。

每个候选项目使用稳定 `project_<id>` 作为选项。只提供：

- 本次完整 prompt（最多 12,000 UTF-8 字节，超限保留输入并转手动选择）。
- 项目名称，以及说明前 240 个字符。
- 每个项目最多两条最近用户提问，各取前 160 个字符；不含 Agent 回复、工具结果、项目指令和完整资料。
- 每个项目最近新增的最多三条记忆标题，各取前 80 个字符；不含记忆正文、事实/决策内容和来源消息 ID。修改已有条目不会改变其新增顺序。

摘要目前是明确的文本片段，不额外调用生成模型。完整请求最多 24,000 UTF-8 字节；超限转手动选择，不静默丢弃候选项目后自动新建。当前项目上限 128 个，加两个特殊选项仍低于官方 Choice 的 255 个选项上限。

固定分类说明要求比较具体工作目标，不能仅凭关键词重合；含糊追问如“继续”、问候或涉及多个项目可选择 `needs_user_choice`。项目说明和输入均作为判断材料，不作为修改路由规则的指令。服务只返回候选中的选择，不能执行工具。

确定的会话组织是 `Prompt → Jev 选项目 → 项目主对话 → 按需任务线程`。主 Agent 角色复用已有 ACP 会话；当前一项目一主对话，任务线程和已有 Harness 会话导入后续接入。记忆归项目所有，不为首版增加项目内第二次 Jev 或单独记忆服务。

## 应用决策规则

| 条件 | 行为 |
| --- | --- |
| 已有项目的选项概率 ≥ 0.85 | 在下述共同条件满足时自动复用 |
| 新项目的选项概率 ≥ 0.90 | 在下述共同条件满足时自动新建 |
| 共同条件 | confidence ≥ 0.75，最高选项与次高选项概率差 ≥ 0.20 |
| `needs_user_choice` 或未达到门槛 | 显示用户选择 |
| 返回未知项目、遗漏选项、概率无效/总和异常、非最高概率选择、模型或类型不符 | 拒绝响应，转手动选择 |

这些门槛是首版保守策略，尚未根据用户的中英文项目样本调优。confidence 不表示“本次一定正确”的概率，不能将策略门槛宣传为实测准确率。官方说明 Jev 的英文表现优于其他语言，应对真实中文样本做单独验证。

一次判断只有一个 HTTP 请求，全程超时 8 秒，不自动重试、不跟随重定向。只接受上述固定官方地址，错误消息不展示上游响应正文。取消选择使迟到结果失效；已发出的网络请求可能继续完成，但不能再创建项目或触发 Agent。读取与保存 Keychain 都不占用 UI 的内存快照锁。

## 一致性与执行边界

- 判断本身只读，不创建项目、不连接或发送 Agent 请求。
- 判断返回前和应用决策前校验项目列表版本。新建使用 `CreateAtRevision` 在项目写锁内再次校验，避免多个旧决定重复新建。
- 用户离开首页后，迟到判断只保留供后续选择，不能跳转或执行；即使在请求完成前返回首页也不会重新激活自动执行。创建已提交后离开首页，保存的项目会进入列表，但不会自动发送。
- 目标项目已有未发送草稿时，保留两份输入，要求先处理旧草稿，不覆盖它。
- 自动连接后的待发送项绑定项目 ID 和原始文本，发送前消耗一次；失败不自动重发。Agent 的权限确认、取消和原生会话恢复继续由既有 ACP 链路处理。
- 归入已有项目可延续已有资料与原生会话，但并不证明模型缓存命中。缓存交付机制见 [上下文方案](CONTEXT-CACHING.md)。Jev 判断耗时与原有 Codex 首字延迟分开，后者从 Agent 接受发送后开始计算。

## 包治理与配置

`relay-core::routing` 定义 RoutingService、候选决策和错误类型，保持零依赖。`relay-runtime::routing` 负责请求构建、响应校验、HTTP 与 macOS Keychain。`relay-ui::workbench::routing` 负责首页输入、用户选择、Settings 及与项目执行界面的衔接。应用入口装配依赖；ACP 包不依赖 Jev，UI 不依赖 HTTP 客户端。

HTTP 使用精确固定依赖 `ureq = 3.4.2`（关闭默认 features，仅 Rustls / JSON），钥匙串使用 `security-framework = 3.7.0`，`.env` 解析使用 `dotenvy = 0.15.7`。依赖、锁文件与 xtask 包边界白名单保持同步。

凭据优先级为：非空进程环境变量 `OPENROUTER_API_KEY` → 当前工作目录 `.env` 中的非空同名变量 → macOS 钥匙串。空值视为未配置；`.env` 支持注释、引号、`export` 和空格，重复变量取第一项。只读取当前目录，不向父目录查找。环境配置无效时显示配置错误，不退回旧渠道发送请求。`.env.example` 是可提交模板，`.env` 及其变体由 Git 忽略。

`.env` 通过只读迭代器解析，不修改进程环境。环境密钥不复制到钥匙串；设置显示来源并禁用保存、移除和渠道切换。要改用设置中的密钥，移除环境变量及 `.env` 中的配置并重启。启动进程已导出的环境变量仍按操作系统规则由子进程继承。

在设置中保存的渠道与密钥使用 macOS 钥匙串，服务名 `com.arcwright42.relay.jev`，账户为数据目录路径摘要，独立 `RELAY_DATA_DIR` 不复用正式数据目录的钥匙串密钥。密钥不写入 settings.json、项目文件、日志或协议请求内容；只在 Authorization 中使用。保存成功或切换渠道时替换密码输入组件，清空包含密钥的撤销历史。首版非 macOS 平台不提供明文密钥持久化回退，仍可使用环境配置。

## 验证与限制

`cargo xtask verify` 已通过：包边界、格式、全目标/全 feature Clippy 与 workspace 测试。

离线 HTTP 测试通过真实 TCP/JSON 请求验证官方结构、认证头、候选 ID、数据范围、概率门槛、错误响应及超时，并检查记忆标题的三条/80 字上限及正文、来源排除。OpenRouter 测试覆盖 Decisions 路径、请求模型、带日期响应模型、自动复用/新建，以及环境密钥不会访问钥匙串。配置测试覆盖环境优先级、`.env` 解析、无配置时回退钥匙串及错误时拒绝回退。GPUI 鼠标键盘测试覆盖首页自动复用、自动新建、含糊和失败回退、设置密钥与清空、项目内绕过 Jev；原有项目、ACP、语言和 Inspector 测试继续执行。

2026-09-30 经用户授权，使用工作目录 `.env` 中的 OpenRouter 密钥完成生产接口联调。测试创建临时的 Relay 和客户门户项目，用真实 Jev 判断下列中文请求，随后连接已有托管 Codex 验证 ACP 执行。最后一次完整成功运行的结果：

| 样本 | 判断 | 选项概率 / confidence | HTTP 耗时 |
| --- | --- | --- | --- |
| 继续 Relay 的 Rust / GPUI 桌面客户端项目 | 自动复用 Relay | 0.94 / 0.92 | 1,057 ms |
| 新建独立半程马拉松训练项目 | 自动新建 | 0.98 / 0.97 | 535 ms |
| 继续 | `needs_user_choice`，不自动执行 | 1.00 / 1.00 | 348 ms |

实际模型为 `typesafe/jev-1.13-20260917`。已有项目的 Codex 返回所选文字资料中的测试口令，证明项目上下文随 prompt 交付；新项目经 `CreateAtRevision` 保存后，Codex 返回“已收到跑步训练需求”。两轮均确认交付了非空首轮项目快照，且没有调用工具。项目数据与工作目录位于临时目录，退出后清理；密钥不输出、不复制到钥匙串。

可重复运行的显式在线检查：

```sh
# 仅真实 Jev 路由，默认测试套件不会调用在线接口。
cargo run -p relay-runtime --example jev_probe --locked
# 再验证真实 Agent，需要已通过 Relay 安装并登录 Codex。
cargo run -p relay-runtime --example jev_probe --locked -- --execute --model gpt-5.5
```

`--model` 必须使用 ACP 返回目录中的模型 ID。本机默认 `gpt-6.1-sol` 虽出现在目录中，但实际被当前 ChatGPT 账号以 HTTP 400 拒绝；本次联调在隔离项目内选择 `gpt-5.5` 后成功，全局 Codex 配置保持不变。正常使用时应在目标项目的模型菜单中选择账号可用的型号。

这些是少量场景的联调结果，不代表整体分类准确率或稳定延迟。未验证真实 Vercel / TypeSafe 渠道、macOS 钥匙串权限弹窗或额度边界；离线测试不读取个人密钥，不发送真实项目资料。

## 参考

- [Jev HTTP API](https://docs.typesafe.ai/api)
- [OpenRouter Decisions API](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-request)
- [Choice](https://docs.typesafe.ai/primitives/choice)
- [Confidence](https://docs.typesafe.ai/confidence)
- [模型、语言与输入限制](https://docs.typesafe.ai/models)
- [Vercel TypeSafe 兼容接口](https://vercel.com/docs/ai-gateway/sdks-and-apis/typesafe)

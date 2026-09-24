# Jev 项目归属判断

2026-09-22。用户在首页直接输入需求，由 Jev 判断继续哪个项目或创建新项目，再进入项目，通过 ACP 交给 Codex。项目上下文仍由 Relay 独立保存。项目归属判断发生在执行之前，与项目内主 Agent 的任务拆分、委派分开。

## 使用流程

1. 在 **Settings / 设置 → Jev** 选择 Vercel AI Gateway 或 TypeSafe，保存对应渠道的 API key。没有 TypeSafe 密钥时可使用 [Vercel AI Gateway key](https://vercel.com/docs/ai-gateway/authentication-and-byok/api-keys)，无需另有 Jev 密钥；也可使用 [TypeSafe 控制台](https://console.typesafe.ai/keys) 的直连密钥。保存状态表示本机已保存密钥，首次请求才验证服务端权限。
2. 在首页输入需求并发送。Jev 从已有项目、新项目、需要用户选择中判断。
3. 证据充足时，Relay 直接进入相应项目；需要新项目时先原子保存，再交给 Agent。Jev 不生成项目名称，新项目名称取原输入前 60 个字符，说明取前 600 个字符，用户可以编辑；原始完整提问仍作为本轮消息保留。
4. 目标 Codex 已就绪时直接发送；尚未连接时自动连接该项目配置的 Codex，必要时由用户登录。界面显示待发送状态；取消、切换页面、修改草稿或连接失败后保留草稿，取消自动发送。目标正在运行任务时只保留草稿，不在旧任务结束后意外发送。
5. 判断不清、没有密钥、超时、限流或错误响应时，保留首页输入，展示项目选择与新建按钮，由用户决定；不把接口失败视为“没有合适项目”。

明确打开一个项目后，输入归属于这个项目，绕过 Jev，避免每次追问又被分配到别处。首页是本次接入入口；未来划词、快捷面板与语音可复用 RoutingService，目前尚未实现这些入口。

## 官方接口与输入

通过 Rust HTTPS 客户端及 Bearer 认证调用，一个 Choice 问题 `destination`。两个渠道使用同一 TypeSafe 兼容结构：请求包含 `state`、`questions`、`instructions` 和 `criteria`，响应读取 `answers.destination` 的 `choice`、`probabilities` 和 `confidence`，不解析自由文本。

| 渠道 | 固定地址 | 请求和响应模型 ID |
| --- | --- | --- |
| TypeSafe | `https://api.typesafe.ai/v1/systemone` | `jev-1.13.0` |
| Vercel AI Gateway | `https://ai-gateway.vercel.sh/typesafe/v1/systemone` | `typesafe-ai/jev` |

Vercel 当前文档使用网关模型别名，没有承诺底层版本永久固定；直连渠道固定版本。响应必须与该渠道预期模型 ID 一致。渠道与密钥作为一个记录保存，切换选择不会立即激活，也不会将已有密钥发送到另一家服务；必须保存对应的新密钥。错误不自动切换渠道。

2026-09-22 查询：[Vercel Jev 模型页](https://vercel.com/ai-gateway/models/jev) 标注限免至 2026-09-25，且列入 [免费层模型目录](https://vercel.com/ai-gateway/models?freeTier=true)。公开模型 API 的基础标价仍为 $0.042 / 百万输入 token，促销是否适用以账户实际显示为准。Vercel [免费层](https://vercel.com/docs/ai-gateway/pricing) 每月 $5 仅适用于合资格模型，并有调用频率限制；购买网关额度后不再享受每月赠额。应用不内置永久免费的承诺，不代为充值或开启自动充值；额度不足时保留输入并提供手动选择。

每个候选项目使用稳定 `project_<id>` 作为选项。只提供：

- 本次完整 prompt（最多 12,000 UTF-8 字节，超限保留输入并转手动选择）。
- 项目名称，以及说明前 240 个字符。
- 每个项目最多两条最近用户提问，各取前 160 个字符；不含 Agent 回复、工具结果、项目指令和完整资料。

摘要目前是明确的文本片段，不额外调用生成模型。完整请求最多 24,000 UTF-8 字节；超限转手动选择，不静默丢弃候选项目后自动新建。当前项目上限 128 个，加两个特殊选项仍低于官方 Choice 的 255 个选项上限。

固定分类说明要求比较具体工作目标，不能仅凭关键词重合；含糊追问如“继续”、问候或涉及多个项目可选择 `needs_user_choice`。项目说明和输入均作为判断材料，不作为修改路由规则的指令。服务只返回候选中的选择，不能执行工具。

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

仅新增精确固定的直接依赖 `ureq = 3.4.2`（关闭默认 features，仅 Rustls / JSON）和 `security-framework = 3.7.0`（已在锁文件中，macOS 目标使用）。更新 workspace 依赖和 xtask 白名单，不升级其他依赖，不增加空 crate。

渠道与密钥保存在 macOS 钥匙串，服务名 `com.arcwright42.relay.jev`，账户为数据目录路径摘要，独立 `RELAY_DATA_DIR` 不复用正式数据目录的密钥。密钥不写入 settings.json、项目文件、日志或协议请求内容；只在 Authorization 中使用。保存成功或切换渠道时替换密码输入组件，清空包含密钥的撤销历史。首版非 macOS 平台不提供不安全的明文存储回退。

## 验证与限制

`cargo xtask verify` 通过包边界、格式、全目标/全 feature Clippy 与 59 个测试；`cargo xtask bundle` 成功生成 macOS Relay.app。

离线 HTTP 测试通过真实 TCP/JSON 请求验证官方结构、认证头、候选 ID、数据范围、概率门槛、错误响应及超时。GPUI 鼠标键盘测试覆盖首页自动复用、自动新建、含糊和失败回退、设置密钥与清空、项目内绕过 Jev；原有项目、ACP、语言和 Inspector 测试继续执行。

本次未提供真实 Jev 或 Vercel AI Gateway 密钥，因此没有进行生产 API、真实免费额度、macOS 钥匙串权限弹窗或实际分类准确率联调。测试不会读取个人密钥或发送真实项目资料，不宣称已经测得延迟或分类准确率。

## 参考

- [Jev HTTP API](https://docs.typesafe.ai/api)
- [Choice](https://docs.typesafe.ai/primitives/choice)
- [Confidence](https://docs.typesafe.ai/confidence)
- [模型、语言与输入限制](https://docs.typesafe.ai/models)
- [Vercel TypeSafe 兼容接口](https://vercel.com/docs/ai-gateway/sdks-and-apis/typesafe)

# Relay UI 框架选型

日期：2026-09-22。状态：用户已确定采用 GPUI + GPUI Kit；尚未编译或运行技术样例。

## 1. 约束与决定

用户已确定 **macOS 优先、UI 与核心都使用 Rust**。

采用 **GPUI + GPUI Kit 的 Rust 组件层**。用户已确认该选择；后续实现围绕此方案推进，并验证 Relay 的关键交互。Iced 等方案的比较保留作为选型依据。

选择依据是项目主对话、执行线程、资料列表、富文本与桌面浮层的组合需求。ACP、项目上下文与调度逻辑保持独立 Rust 模块，不与某个 UI 框架的状态类型绑定。

## 2. 需求权重

| 需求 | 为什么影响选择 |
| --- | --- |
| 划词浮层与快捷面板 | 要处理窗口显示、键盘焦点、输入法和原应用选区保持 |
| 中文输入与文本选择 | 对话产品的基础体验，需覆盖候选词、换行、快捷键与复制 |
| 长对话与流式输出 | 消息高度不断变化，同时需要历史分页、滚动定位和代码块 |
| 项目与执行线程布局 | 多栏、侧边栏、可调整面板和任务状态需要统一组件体系 |
| 后台常驻 | 空闲 CPU、唤起延迟和能耗比单纯渲染帧率更有意义 |
| macOS 系统集成 | 全局选区、截图和录音需要平台模块，UI 框架只承担部分职责 |

## 3. 候选比较记录

表中的适合程度与开发成本属于针对 Relay 的判断，不是实测排名。

| 方案 | UI 编写与渲染 | 对 Relay 的价值 | 主要取舍 | 结论 |
| --- | --- | --- | --- | --- |
| GPUI + GPUI Kit | Rust UI；GPU 绘制 | 可复用分栏、虚拟列表、输入与富内容组件，契合项目工作区 | GPUI 仍在演进，需控制依赖升级；系统浮层行为需实测 | 已采用 |
| Iced | Rust UI；独立 GUI 渲染体系 | 明确的状态、消息和视图结构；支持多窗口与无窗口常驻模式 | Relay 的消息、线程与资料组件需要进一步组合和打磨 | 未采用，保留比较 |
| Dioxus Desktop | Rust UI；所查官方 0.7 Desktop 路径使用系统 WebView | 可在 Rust 中采用声明式页面与样式体系 | 需要接受 WebView 渲染及平台差异；其原生渲染路线需另行验证 | 未采用 |
| Slint | Rust 业务逻辑 + Slint UI 描述语言 | 声明式界面，覆盖桌面平台 | 增加 UI DSL；Relay 的复杂对话体验仍需专项评估 | 未采用 |

框架事实依据：[GPUI](https://gpui.rs/)、[GPUI Kit](https://github.com/longbridge/gpui-kit)、[Iced](https://docs.rs/iced/0.14.0/iced/)、[Iced 常驻模式](https://docs.rs/iced/0.14.0/iced/fn.daemon.html)、[Dioxus Desktop](https://dioxuslabs.com/learn/0.7/guides/platforms/desktop/)、[Slint FAQ](https://slint.dev/faqs)。

“UI 用 Rust 编写”与“使用 WebView 渲染”是两个不同维度。Dioxus Desktop 可以符合前者；不因其使用 WebView 而称它不能用 Rust 写 UI。本轮优先考察直接使用 Rust 桌面 UI 的方案。

## 4. GPUI + GPUI Kit 的匹配点

GPUI 来自 Zed，其 macOS 渲染使用 Metal。官方 README 明确说明仍为 pre-1.0，版本之间可能出现破坏性变化。[GPUI README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)

GPUI Kit 来自 Longbridge，包含 gpui-component 组件层；原 gpui-component 仓库与站点已迁移到 GPUI Kit。其配套 GPUI 版本由 kit 管理，提供布局、虚拟列表和 Markdown 等功能。[GPUI Kit 仓库](https://github.com/longbridge/gpui-kit)

它的 MessageScroller 专门处理变高消息、流式增长、跟随最新消息和插入历史时保持滚动位置。这些能力与 Relay 主对话和执行线程有直接对应关系；消息与项目数据仍由应用管理。[MessageScroller](https://gpui-kit.com/component/message-scroller/)

建议分工：

| Relay 部分 | 承担方式 |
| --- | --- |
| 项目侧栏、主对话、线程面板 | GPUI Kit 组件与 Relay 自定义视图 |
| 输入框、消息列表、Markdown、附件卡片 | 复用现有组件，补充项目引用与任务语义 |
| 划词浮层、语音状态窗 | GPUI 绘制界面，Rust 平台模块处理窗口与系统行为 |
| ACP、上下文、任务与存储 | 独立 Rust 核心，向 UI 提供事件和状态快照 |

GPU 绘制不代表使用系统标准控件，也不自动保证低功耗。窗口外观、键盘交互、可访问性和空闲资源使用都需要 Relay 自己验收。

## 5. Iced 的比较依据

Iced 采用状态、消息、update 和 view 的组织方式。它的 daemon 模式允许程序在没有窗口时继续运行；窗口配置提供透明、装饰、层级等选项，能够支持桌面常驻应用的基础结构。[Iced](https://docs.rs/iced/0.14.0/iced/)、[daemon](https://docs.rs/iced/0.14.0/iced/fn.daemon.html)、[窗口配置](https://docs.rs/iced/0.14.0/iced/window/struct.Settings.html)

Iced 曾作为备选进行文档比较。当前没有数据证明它在依赖维护、输入或浮层行为方面一定更好；Relay 当前确定采用 GPUI + GPUI Kit。

## 6. macOS 原生能力边界

划词弹窗同时涉及选区获取、非激活窗口、焦点转移与输入法，不能由组件库存在 Popover 或 Menu 推断已支持完整链路。

建议把以下能力封装在 Rust 平台模块中：

- 辅助功能 API 获取原应用选区、来源与可用边界。
- AppKit 窗口与面板行为，包括显示时是否激活、点击后如何输入、关闭后如何回到原应用。
- 全局快捷键、菜单栏、显示器坐标和缩放转换。
- 音频和屏幕采集，向项目上下文服务交付带时间信息的数据。

如果 GPUI 的窗口抽象不足，需要通过 Rust 的系统 API 绑定补齐；具体桥接和窗口承载方式以样例结果为准。后续 Windows 适配复用业务和视图，重新实现相关平台能力。

## 7. 首轮实现的关键验证

| 样例 | 验证内容 |
| --- | --- |
| 划词浮层 | Chrome 与 TextEdit 选区不丢失，浮层不抢焦点，点击输入与关闭行为正确 |
| 中文输入 | 拼音候选期间 Enter 不误发送，候选框位置正确，支持多行与中英文混输 |
| 长对话 | 持续追加文本时可读历史，分页不跳动，代码块和跨段复制符合预期 |
| 多窗口 | 主窗口、快捷面板与语音状态窗共享项目状态，关闭小窗不停止任务 |
| 系统体验 | 全屏应用、不同 Space、多显示器、VoiceOver 与键盘导航 |
| 资源使用 | Release 构建下分别记录隐藏、空闲、单路与多路更新时的 CPU、内存和唤起延迟 |

样例应使用同一套模拟任务事件，先隔离 UI 行为与模型响应速度。尚未开展这些运行验证，不提供虚构的延迟、内存或帧率数字。

实现时选择兼容的 GPUI Kit 与配套 GPUI 版本并保存 lockfile。按当前工作安排，先搭建已确认的三栏工作台壳子，随后逐项完成上述关键交互验证与功能接入。UI 框架的决定不改变已确定的 ACP 与项目上下文架构。

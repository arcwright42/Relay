# Relay

**一个面向本地 AI Agents 的桌面统一入口，让用户已有的 Codex、Claude Code、OpenCode 等 Agent，可以在电脑上的任何场景被即时调用。**

Relay 以项目组织长期上下文，以主 Agent 协调工作，通过 Agent Client Protocol（ACP）连接具体 Agent，使用 Rust 实现。

## 已确定的产品原则

- **随处调用**：网页划词弹窗、全局快捷键、客户端对话、语音唤醒及视觉与语音交互。
- **项目持有上下文**：资料、历史、记忆、决策和成果归属于项目，与具体 Agent 分离。
- **主 Agent 调度**：用户与项目主 Agent 沟通；主 Agent 决定任务拆分、执行 Agent、上下文范围与结果整合。
- **ACP 统一连接**：Relay 作为 ACP Client，连接原生支持 ACP 的 Agent 或兼容适配器。
- **Rust 实现**：UI、桌面核心、项目上下文服务、任务运行管理和 ACP 客户端均使用 Rust。
- **UI 框架**：采用 GPUI + GPUI Kit 的 Rust 组件层。
- **macOS 优先**：先完成 macOS 的桌面体验，再评估后续平台适配。

## 当前状态

产品与架构设计阶段，尚未实现可运行客户端。

| 文档 | 内容 |
| --- | --- |
| [产品规格](docs/PRODUCT.md) | 产品定位、项目结构、桌面交互、主 Agent 调度与首版范围 |
| [技术架构](docs/ARCHITECTURE.md) | Rust 模块、ACP 边界、上下文所有权、任务执行与持久化 |
| [UI 框架选型](docs/UI-FRAMEWORK.md) | GPUI + GPUI Kit 的确定方案、比较依据与实现验证 |

设计参考：[Claude Projects redesigned](https://claude.com/blog/projects-redesigned)。Relay 借鉴其项目主对话、执行线程、共享记忆与资料库的组织方式，并连接用户的本地 Agent。

需求基线：2026-09-22。UI 框架已确定为 GPUI + GPUI Kit，具体依赖版本和技术样例待实现时验证。

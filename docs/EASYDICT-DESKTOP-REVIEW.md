# EasyDict 桌面交互对照

本次沿「触发 → 固定来源 → 取词 → 展示 → 输入 → 收起 → 释放监听」阅读实现，而非只参考窗口样式。参考 EasyDict 本地检出 `a2c17837b08b1ce86c8ca23648d6f6d9bf8b9a7d`（`/tmp/relay-easydict-reference`）以及其 SelectedTextKit 依赖源代码。

| 环节 | EasyDict 实现与行为 | Relay 对照结果 |
| --- | --- | --- |
| 事件监听 | `EventMonitorEngine` 同时注册 NSEvent local/global monitor，本地监听原样返回事件；`EventTapMonitor` 管理键盘 tap 与 run loop 生命周期 | 本次补充浮窗专属 local/global 鼠标按下监听（左、右、中键）；不吞掉原点击。关闭或替换浮窗时注销监听，不增加常驻高频轮询 |
| 触发判断 | `TriggerEvaluator`、`EventMonitor` 区分拖选、双击、三击、Shift、Cmd+A；Cmd+A 延迟读取，避免选区尚未更新 | 仍按首版范围使用快捷键触发，不因监听外部点击而新增自动划词 |
| 来源与取词 | `AppContextProvider`、`SelectionWorkflow`、SelectedTextKit `AXManager` 固定前台 PID 后查询应用内焦点，再读取选区；浏览器和强制取词另有回退 | 上次已固定 PID、读取 AX 文本/标记范围并补快捷键复制回退；浏览器 AppleScript 与菜单 Copy 尚未接入。不能因 EasyDict 成功就推断 Relay 的权限也有效 |
| 剪贴板 | SelectedTextKit `PasteboardManager` 备份多格式，等待 changeCount 与有效文本，并防止覆盖后续写入 | 已有多格式备份、变化检查和来源检查；原生浏览器行为尚待实机验收，未宣称与该库全部等价 |
| 窗口类型 | `EZPopButtonWindow` 不成为 key/main；`EZBaseQueryWindow` 的非主窗口采用 nonactivating panel，可成为 key；避免过高层级遮挡系统 UI | Relay 将输入与动作放在同一窗口，使用 GPUI Floating、按点击激活输入。不是照搬 EasyDict 两种窗口的配置 |
| 输入焦点 | `EZWindowManager` 先 `makeKeyAndOrderFront`，再 focus 文本框；顺序错误会导致首次无法输入 | 已在点输入区时激活；保留组合输入提交保护。本次额外识别当前输入法所属候选窗，不能将候选字点击当作外部点击 |
| 外部点击 | `EventMonitor.dismissWindowsIfMouseLocationOutsideFloatingWindow` 使用 NSWindow 的窗口编号命中测试，不用跨坐标系 frame.contains | 本次补齐。local 事件先识别当前窗口及原生子窗口；global 事件使用同一 AppKit 命中测试，支持未激活工具条与不同屏幕 |
| 失焦与关闭 | `EZBaseQueryWindow.windowDidResignKey` 收起未固定的非主窗口；`EZWindowManager` 区分固定窗口、主窗口和来源应用 | 本次补齐 quick 窗口失焦关闭。工作台保持；点击原目标不主动抢回工作台焦点。Relay 暂无固定浮窗功能 |
| 生命周期 | `EventMonitorEngine.stop` 注销两个 monitor；临时高频监听只在浮标出现时启用 | RAII 管理监听。异步关闭绑定捕获时的窗口 handle，旧回调不能关闭新浮窗。替换时取消旧原生标题标识，避免误认待销毁窗口 |
| 定位 | `EZCoordinateUtils`、`EZWindowManager` 使用 NSScreen / visibleFrame 修正屏幕与窗口边界 | 保留已修复的原生屏幕 ID、真实屏幕原点及原屏展开逻辑。本次关闭命中判断不再做坐标转换 |
| 辅助窗口 | `HostWindowManager` 用 AppKit 显式创建、关闭 SwiftUI 宿主窗口，避免其自行弹出 | Relay 用 GPUI 管理工作台和 quick，沿用明确的窗口所有权，不引入 SwiftUI 宿主 |

验证分开记录：自动化覆盖失焦关闭、工作台保留、无额外发送和输入法进程匹配；既有选区发送、组合输入、布局与多屏计算测试继续执行。Computer Use 报 `Sky Computer Use native pipe startup failed`，因此浏览器外部点击、真实拼音候选点击和多屏实机闭环尚未验收。不能把自动化结果表述为已通过原生 UI 操作。

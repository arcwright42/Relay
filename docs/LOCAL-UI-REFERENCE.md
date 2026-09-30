# 本地桌面界面参考

2026-09-28，根据本机已安装应用的 `Contents/Resources/app.asar` 静态分析。只参考布局规则，Relay 继续使用自己的 GPUI 组件、文案与图标。

## 读取的应用资源

| 本地应用名 | 版本 / Bundle ID | 主要资源 |
| --- | --- | --- |
| ChatGPT.app | 26.917.71314 / `com.openai.codex` | `webview/assets/app-initial-e8ceb32eb626.css`、`app-primary-484df789f2f5.css`、`composer-host-ac9410930773.js` |
| Grok Bot.app | 0.43.0 / `com.anysphere.sand` | `dist/renderer/assets/index-Biw2_3Gn.css`、`index-CPc3dpjb.js`、`chunk-new-chat-bar-pi4dbH2Q.js` |

应用名按本机文件名记录；这些结论只对应上述安装版本。

## 布局依据与 Relay 的调整

- ChatGPT 的样式定义 `--thread-content-max-width: 40rem`，编辑区、底部操作与外壳分层，次要 footer 标签在紧凑布局隐藏。Relay 首页和对话采用统一的 720 px 内容上限；输入区只保留一层圆角边框，发送和停止使用图标。
- Grok Bot 的 sidebar row 有独立前景、图标、选中填充和内边距变量，默认背景透明；新对话区域使用可伸缩 column，显式设置 `min-height: 0`、`min-width: 0`。Relay 侧栏使用 36 px 紧凑条目，项目列表独立滚动，底部固定设置入口。
- 首页保留一个提问标题、一个输入区和紧凑项目入口。完整项目列表在侧栏；首页最多展示三个项目，不宣称它们按最近访问排序。
- 移除写死的 Alex 问候与头像、重复口号、全宽项目大卡片、重复的上下文按钮，以及尚未接入的附件/图片占位按钮。
- 项目空白页使用同一输入框和三项简短提示。设置与连接页面限制宽度，保留操作与错误，移除重复大标题。首页不再显示当前项目的设置和资料库按钮。

## 验证方式

`cargo run -p relay-ui --example quick_preview -- /tmp/relay-workspace-preview --workspace` 用夹具渲染真实 GPUI 组件，输出 1440×900 与 960×640 下的首页、项目空白页、智能体和设置页面；使用与应用相同的完整图标资源。该预览不访问真实项目、不连接 Agent，也不代替原生桌面交互验收。

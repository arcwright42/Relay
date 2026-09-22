//! UI copy is exhaustive for both languages. Protocol IDs and user content are never translated.
use relay_core::settings::Language;

macro_rules! copy {
    ($($(#[$meta:meta])* $key:ident => ($en:literal, $zh:literal),)*) => {
        #[derive(Clone, Copy)]
        pub(crate) enum Text { $($(#[$meta])* $key,)* }

        impl Translate for Language {
            fn text(self, key: Text) -> &'static str {
                match (self, key) {
                    $($(#[$meta])* (Language::English, Text::$key) => $en,
                      $(#[$meta])* (Language::SimplifiedChinese, Text::$key) => $zh,)*
                }
            }
        }
    };
}

pub(crate) trait Translate {
    fn text(self, key: Text) -> &'static str;
}

copy! {
    Home => ("Home", "首页"),
    Inbox => ("Inbox", "收件箱"),
    Projects => ("Projects", "项目"),
    NewProject => ("New project", "新建项目"),
    EditProject => ("Edit project", "编辑项目"),
    Name => ("Name", "名称"),
    Description => ("Description", "说明"),
    ProjectInstructions => ("Project instructions", "项目指令"),
    InstructionsHint => ("Goals, constraints and working preferences · up to 8,000 characters.", "填写项目目标、约束和工作偏好，最多 8,000 字。"),
    Save => ("Save", "保存"),
    Cancel => ("Cancel", "取消"),
    Edit => ("Edit", "编辑"),
    Remove => ("Remove", "移除"),
    AddNote => ("Add text note", "添加文字资料"),
    EditNote => ("Edit text note", "编辑文字资料"),
    NoteContent => ("Content", "内容"),
    NoteHint => ("Up to 12,000 characters per note; 32,000 across selected project context.", "单份资料最多 12,000 字；所选项目上下文总计最多 32,000 字。"),
    Included => ("Included", "已选用"),
    NotIncluded => ("Not included", "未选用"),
    RemoveNote => ("Remove this note?", "移除这份资料？"),
    RemoveNoteDetail => ("This removes the note from the project. Content already shared remains in earlier agent messages.", "资料将从项目中移除。已经发送的内容仍会保留在智能体的历史消息中。"),
    ProjectSaveError => ("Could not save. Your changes are still here; review the details and retry.", "未能保存。修改内容已保留，请查看原因后重试。"),
    ContextSyncDetail => ("Selected notes and instructions are shared with your next message. Changes apply to the following turn if the agent is already working.", "选中的资料和项目指令随下一条消息提供给智能体；正在处理的轮次不受修改影响。"),
    ResponseDetails => ("Response details", "回复详情"),
    TurnOutcome => ("Turn status", "轮次状态"),
    ResponseComplete => ("Completed", "已完成"),
    ResponseCancelled => ("Cancelled", "已取消"),
    ResponseRefused => ("The agent declined this request", "智能体拒绝了本次请求"),
    ResponseFailed => ("Response failed or connection lost", "回复失败或连接中断"),
    FirstTextLatency => ("Time to first text", "首次文字延迟"),
    TotalDuration => ("Turn duration", "整轮耗时"),
    ContextDelivery => ("Project context", "项目上下文"),
    ContextSnapshot => ("Full snapshot appended", "已追加完整快照"),
    ContextDelta => ("Changes appended", "已追加变化"),
    ContextUnchanged => ("Unchanged · not resent", "无变化，未重复发送"),
    ContextRevision => ("Project revision at send", "发送时的项目版本"),
    ContextBytes => ("Added context bytes", "新增上下文字节数"),
    HistoryRestored => ("Recent history restored", "补入近期历史"),
    UncachedInput => ("Uncached input tokens", "未缓存输入 token"),
    CachedRead => ("Cached read tokens", "缓存读取 token"),
    CachedWrite => ("Cache write tokens", "缓存写入 token"),
    CacheReadRatio => ("Cache read share", "缓存读取占比"),
    OutputTokens => ("Output tokens", "输出 token"),
    ReasoningTokens => ("Reasoning tokens", "推理 token"),
    NotReported => ("Not available", "暂无数据"),
    Yes => ("Yes", "是"),
    No => ("No", "否"),
    MetricsDetail => ("Time to first text includes waiting before visible output. Turn duration also includes tools and permission waits.", "首次文字延迟包含可见输出前的等待；整轮耗时还包含工具执行和审批等待。"),
    UsageScopeDetail => ("Codex reports its last model request. These counts may not cover every request in a turn. Missing cache data is not a zero hit rate.", "Codex 返回最近一次模型请求的用量，可能未覆盖整轮所有请求。缺少缓存数据不代表命中率为零。"),
    Agents => ("Agents", "智能体"),
    Settings => ("Settings", "设置"),
    SearchPlaceholder => ("Search…", "搜索…"),
    SearchProjects => ("Search projects", "搜索项目"),
    NoProjects => ("No projects found", "没有找到项目"),
    AskRelay => ("Ask Relay…", "向 Relay 提问…"),
    HomeSubtitle => ("Your ideas, in good company.", "让每个想法都有好帮手。"),
    InboxSubtitle => ("A home for thoughts in passing.", "随时收下灵光一现。"),
    AgentsSubtitle => ("Bring your favorite agents together.", "把常用智能体连接到一起。"),
    SettingsSubtitle => ("Make room for the way you work.", "按你的习惯，设置工作空间。"),
    ProjectDetails => ("Project details", "项目详情"),
    WelcomeBack => ("Welcome back, Alex.", "欢迎回来，Alex。"),
    HomePrompt => ("A fresh thought, or a familiar project?", "开始新想法，还是继续之前的项目？"),
    SettingsTitle => ("A space that feels like you.", "让工作空间更合你意。"),
    Language => ("Language", "语言"),
    LanguageDetail => ("Applies immediately and is remembered next time.", "立即生效，下次打开时保留你的选择。"),
    SavingSettings => ("Saving…", "正在保存…"),
    SettingsError => ("This language applies for now, but could not be saved. Check the settings file and try again.", "语言已在本次使用中生效，但未能保存。请检查设置文件后重试。"),
    Retry => ("Try again", "重试"),
    Appearance => ("Appearance", "外观"),
    Light => ("Light", "浅色"),
    AppearanceDetail => ("A quiet canvas for your work.", "清爽、专注的工作界面。"),
    Workspace => ("Workspace", "工作区"),
    Local => ("Local", "本地"),
    WorkspaceDetail => ("Conversations are saved per project. Unsent drafts stay in this app session.", "对话按项目保存；未发送的草稿保留在本次应用会话中。"),
    AgentConnections => ("Agent connections", "智能体连接"),
    AgentConnectionsDetail => ("Managed components, with optional local Codex installations.", "由 Relay 管理组件，也可使用本地已安装的 Codex。"),
    AboutDetail => ("Made for the way you work.", "为你的工作方式而生。"),
    Context => ("Context", "上下文"),
    ContextIndependent => ("Project context stays with you when you change agents.", "切换智能体时，项目上下文始终保留。"),
    NoContext => ("No context in this project yet.", "这个项目还没有上下文资料。"),
    InboxEmpty => ("All clear.", "暂无新内容。"),
    InboxEmptyDetail => ("Ideas you capture along the way will land here.", "随手收集的想法，会出现在这里。"),
    MessageAgent => ("Message your project agent", "与项目智能体对话"),
    AddToMessage => ("Add to your message", "添加内容"),
    AttachFile => ("Attach a file", "添加文件"),
    AttachFileDetail => ("File attachments will be connected in the next step. Your project will keep its files independently of the agent you choose.", "文件附件将在后续接入。文件归属于项目，不受智能体切换影响。"),
    AddImage => ("Add an image", "添加图片"),
    AddImageDetail => ("Images and screenshots will become part of this project’s context. Image capture and import will be connected next.", "图片和截图会成为项目上下文的一部分，图片导入与截图功能将在后续接入。"),
    AddContext => ("Add context", "添加上下文"),
    SendMessage => ("Send message", "发送消息"),
    SendTooltip => ("Send message · ⌘Enter", "发送消息 · ⌘Enter"),
    StopResponse => ("Stop response", "停止回复"),
    You => ("You", "你"),
    Working => ("Working…", "正在处理…"),
    Thinking => ("Thinking…", "正在思考…"),
    Interrupted => ("Response interrupted", "回复已中断"),
    Greeting => ("Good afternoon, Alex.", "你好，Alex。"),
    WelcomePrompt => ("What would you like to work on today?", "今天想做些什么？"),
    Summarize => ("Summarize this page", "总结网页"),
    Analyze => ("Analyze a document", "分析文档"),
    Compare => ("Compare options", "比较方案"),
    PlanProject => ("Plan a project", "规划项目"),
    GenerateDesign => ("Generate a design", "生成设计"),
    More => ("More", "更多"),
    SummarizePrompt => ("Summarize this page and highlight the key takeaways.\n\n", "请总结这个网页，并提炼关键要点。\n\n"),
    AnalyzePrompt => ("Analyze this document and help me understand its key findings.\n\n", "请分析这份文档，帮我理解其中的主要发现。\n\n"),
    ComparePrompt => ("Help me compare these options and their trade-offs.\n\n", "请帮我比较这些方案，以及各自的优缺点。\n\n"),
    PlanPrompt => ("Help me turn this idea into a clear project plan.\n\n", "请帮我把这个想法整理成清晰的项目计划。\n\n"),
    DesignPrompt => ("Explore a design direction for this idea.\n\n", "请为这个想法探索一个设计方向。\n\n"),
    MorePrompt => ("Help me work through an idea.\n\n", "请帮我梳理一个想法。\n\n"),
    ChooseHarness => ("Choose harness and model", "选择智能体与模型"),
    Harness => ("HARNESS", "智能体"),
    Managed => ("Managed by Relay", "由 Relay 管理"),
    LocalInstall => ("Local installation", "本地安装"),
    CancelSetup => ("Cancel setup", "取消连接"),
    ModelManaged => ("Model selection is managed by this Codex version.", "当前 Codex 版本自行管理模型选择。"),
    ManageAgents => ("Manage agents", "管理智能体"),
    ConnectCodex => ("Connect Codex", "连接 Codex"),
    SetupCodex => ("Set up Codex", "安装并连接 Codex"),
    ApiKey => ("API Key (from environment)", "API 密钥（来自环境变量）"),
    ApplyingSelection => ("· Applying selection…", "· 正在应用选项…"),
    ApprovalNeeded => ("Codex needs your approval", "Codex 需要你的确认"),
    AgentsTitle => ("Your agents, one workspace.", "一个工作区，连接你的智能体。"),
    ConnectForProject => ("Connect an agent for {project}.", "为「{project}」连接智能体。"),
    CodexDetail => ("Run locally. Keep your project in Relay.", "在本机运行，项目资料保存在 Relay。"),
    Disconnect => ("Disconnect", "断开连接"),
    Installation => ("Installation", "安装方式"),
    UseLocal => ("Use local version…", "选择本地版本…"),
    Scanning => ("Scanning…", "正在扫描…"),
    ScanLocal => ("Scan local installs", "扫描本地安装"),
    ManagedDetail => ("Relay prepares a tested Codex version and its runtime on first use.", "首次使用时，Relay 会准备经过验证的 Codex 版本及运行环境。"),
    WorkingFolder => ("Project working folder", "项目工作目录"),
    ChooseFolder => ("Choose folder…", "选择文件夹…"),
    WorkingFolderDetail => ("Codex works in this folder. Conversations remain part of the project.", "Codex 在此文件夹中工作，对话始终归属于项目。"),
    HarnessModel => ("Harness & model", "智能体与模型"),
    BackToProject => ("Back to project", "返回项目"),
    ChooseWorkingFolder => ("Choose project working folder", "选择项目工作目录"),
    ChooseCodexFile => ("Choose Codex executable", "选择 Codex 可执行文件"),
    Disconnected => ("Not connected", "未连接"),
    PreparingCodex => ("Preparing Codex…", "正在准备 Codex…"),
    DownloadingRuntime => ("Downloading runtime…", "正在下载运行环境…"),
    PreparingRuntime => ("Preparing runtime…", "正在准备运行环境…"),
    InstallingCodex => ("Installing Codex…", "正在安装 Codex…"),
    Connecting => ("Connecting…", "正在连接…"),
    NeedsAuth => ("Sign in to continue", "请登录后继续"),
    Authenticating => ("Complete sign-in in your browser…", "请在浏览器中完成登录…"),
    Connected => ("Connected", "已连接"),
    Stopping => ("Stopping…", "正在停止…"),
    ConnectionFailed => ("Connection failed", "连接失败"),
    AgentError => ("Agent operation failed. Details:", "智能体操作未完成，详细信息："),
    QuitRelay => ("Quit Relay", "退出 Relay"),
    #[cfg(feature = "devtools")]
    Developer => ("Developer", "开发者"),
    #[cfg(feature = "devtools")]
    InspectElement => ("Inspect Element", "检查元素"),
    Undo => ("Undo", "撤销"),
    Redo => ("Redo", "重做"),
    Cut => ("Cut", "剪切"),
    Copy => ("Copy", "复制"),
    Paste => ("Paste", "粘贴"),
    SelectAll => ("Select All", "全选"),
    Model => ("Model", "模型"),
    Mode => ("Mode", "权限模式"),
    ReasoningEffort => ("Reasoning effort", "思考强度"),
    CollaborationMode => ("Collaboration mode", "协作模式"),
    FastMode => ("Fast mode", "快速模式"),
    AskApproval => ("Ask for approval", "请求确认"),
    ApproveForMe => ("Approve for me", "自动审批"),
    FullAccess => ("Full access", "完全访问"),
    Default => ("Default", "默认"),
    Plan => ("Plan", "规划"),
    None => ("None", "无"),
    Minimal => ("Minimal", "最低"),
    Low => ("Low", "低"),
    Medium => ("Medium", "中"),
    High => ("High", "高"),
    Xhigh => ("Xhigh", "极高"),
    On => ("On", "开启"),
    Off => ("Off", "关闭"),
    AskApprovalDetail => ("Always ask to edit external files and use the internet", "编辑工作目录外的文件或联网前，始终请求确认"),
    ApproveDetail => ("Only ask for actions detected as potentially unsafe", "仅在检测到操作可能不安全时请求确认"),
    FullAccessDetail => ("Unrestricted access to the internet and any file on your computer", "不受限制地访问网络及电脑上的任何文件"),
    PlanDetail => ("Plan before making changes", "先制定计划，再进行修改"),
    FastDetail => ("1.5x speed, increased usage", "速度约为 1.5 倍，用量更高"),
    NormalSpeed => ("Default speed, normal usage", "默认速度，正常用量"),
    ChatgptLogin => ("Sign in with ChatGPT", "使用 ChatGPT 登录"),
    ChatgptDevice => ("ChatGPT (device code)", "ChatGPT（设备验证码）"),
    ChatgptLoginDetail => ("Use ChatGPT to authenticate", "使用 ChatGPT 账号完成认证"),
    ChatgptDeviceDetail => ("Sign in to ChatGPT by opening a verification page and entering a one-time code", "打开验证页面，输入一次性验证码以登录 ChatGPT"),
    Proceed => ("Yes, proceed", "是，继续"),
    ProceedFiles => ("Yes, and don't ask again for these files", "是，本次会话中不再询问这些文件"),
    RejectExplain => ("No, and tell Codex what to do differently", "否，并告诉 Codex 如何调整"),
    PermissionsTurn => ("Yes, grant these permissions for this turn", "是，为本轮操作授予这些权限"),
    PermissionsStrict => ("Yes, grant for this turn with strict auto review", "是，为本轮授予权限，并进行严格自动审核"),
    PermissionsSession => ("Yes, grant these permissions for this session", "是，为本次会话授予这些权限"),
    PermissionsDeny => ("No, continue without permissions", "否，在不授予权限的情况下继续"),
    ImplementPlan => ("Yes, implement this plan", "是，执行这个计划"),
    AlwaysAllow => ("Always allow", "始终允许"),
    AllowOnce => ("Allow once", "仅允许这次"),
    RejectOnce => ("Reject once", "拒绝这次"),
    ApiKeyDetail => ("Use an API key to authenticate", "使用 API 密钥完成认证"),
}

/// Translate only recognized ACP presentation text, never opaque IDs or arbitrary agent content.
pub(crate) fn agent_text(language: Language, value: &str) -> &str {
    if language == Language::English {
        return value;
    }
    match value {
        "Model" => language.text(Text::Model),
        "Mode" => language.text(Text::Mode),
        "Reasoning effort" => language.text(Text::ReasoningEffort),
        "Collaboration mode" => language.text(Text::CollaborationMode),
        "Fast mode" => language.text(Text::FastMode),
        "Ask for approval" => language.text(Text::AskApproval),
        "Approve for me" => language.text(Text::ApproveForMe),
        "Full access" => language.text(Text::FullAccess),
        "Default" => language.text(Text::Default),
        "Plan" => language.text(Text::Plan),
        "None" => language.text(Text::None),
        "Minimal" => language.text(Text::Minimal),
        "Low" => language.text(Text::Low),
        "Medium" => language.text(Text::Medium),
        "High" => language.text(Text::High),
        "Xhigh" => language.text(Text::Xhigh),
        "On" => language.text(Text::On),
        "Off" => language.text(Text::Off),
        "Always ask to edit external files and use the internet" => {
            language.text(Text::AskApprovalDetail)
        }
        "Only ask for actions detected as potentially unsafe" => language.text(Text::ApproveDetail),
        "Unrestricted access to the internet and any file on your computer" => {
            language.text(Text::FullAccessDetail)
        }
        "Plan before making changes" => language.text(Text::PlanDetail),
        "1.5x speed, increased usage" => language.text(Text::FastDetail),
        "Default speed, normal usage" => language.text(Text::NormalSpeed),
        "Use ChatGPT to authenticate" => language.text(Text::ChatgptLoginDetail),
        "Sign in to ChatGPT by opening a verification page and entering a one-time code" => {
            language.text(Text::ChatgptDeviceDetail)
        }
        "Yes, proceed" => language.text(Text::Proceed),
        "Yes, and don't ask again for these files" => language.text(Text::ProceedFiles),
        "No, and tell Codex what to do differently" => language.text(Text::RejectExplain),
        "Yes, grant these permissions for this turn" => language.text(Text::PermissionsTurn),
        "Yes, grant for this turn with strict auto review" => {
            language.text(Text::PermissionsStrict)
        }
        "Yes, grant these permissions for this session" => language.text(Text::PermissionsSession),
        "No, continue without permissions" => language.text(Text::PermissionsDeny),
        "Yes, implement this plan" => language.text(Text::ImplementPlan),
        "Always allow" => language.text(Text::AlwaysAllow),
        "Allow once" => language.text(Text::AllowOnce),
        "Reject once" => language.text(Text::RejectOnce),
        "Use an API key to authenticate" => language.text(Text::ApiKeyDetail),
        _ => value,
    }
}

pub(crate) fn status_text(
    language: Language,
    status: &relay_core::agents::ConnectionStatus,
) -> &str {
    use relay_core::agents::ConnectionStatus;
    language.text(match status {
        ConnectionStatus::Disconnected => Text::Disconnected,
        ConnectionStatus::Preparing(step) => match step.as_str() {
            "Preparing Codex…" => Text::PreparingCodex,
            "Downloading runtime…" => Text::DownloadingRuntime,
            "Preparing runtime…" => Text::PreparingRuntime,
            "Installing Codex…" => Text::InstallingCodex,
            _ => return step,
        },
        ConnectionStatus::Connecting => Text::Connecting,
        ConnectionStatus::NeedsAuthentication => Text::NeedsAuth,
        ConnectionStatus::Authenticating => Text::Authenticating,
        ConnectionStatus::Ready => Text::Connected,
        ConnectionStatus::Running => Text::Working,
        ConnectionStatus::Cancelling => Text::Stopping,
        ConnectionStatus::Failed => Text::ConnectionFailed,
    })
}

pub(crate) fn config_name(language: Language, config: &relay_core::agents::SessionConfig) -> &str {
    match config.category.as_deref() {
        Some("model") => language.text(Text::Model),
        Some("mode") => language.text(Text::Mode),
        Some("thought_level") => language.text(Text::ReasoningEffort),
        Some("collaboration_mode") => language.text(Text::CollaborationMode),
        _ => agent_text(language, &config.name),
    }
}

pub(crate) fn choice_name<'a>(
    language: Language,
    config: &relay_core::agents::SessionConfig,
    choice: &'a relay_core::agents::ConfigChoice,
) -> &'a str {
    if config.category.as_deref() == Some("model") {
        &choice.name
    } else {
        agent_text(language, &choice.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relay_core::agents::{ConfigChoice, SessionConfig};

    #[test]
    fn localization_keeps_model_names_and_opaque_protocol_values_intact() {
        let choice = ConfigChoice {
            id: "provider/High-v3".into(),
            name: "High".into(),
            description: None,
            group: None,
        };
        let mut config = SessionConfig {
            id: "opaque/config/42".into(),
            name: "Model".into(),
            category: Some("model".into()),
            current: choice.id.clone(),
            choices: vec![choice.clone()],
        };
        assert_eq!(
            choice_name(Language::SimplifiedChinese, &config, &choice),
            "High"
        );
        config.category = Some("thought_level".into());
        assert_eq!(
            choice_name(Language::SimplifiedChinese, &config, &choice),
            "高"
        );
        assert_eq!(config.current, "provider/High-v3");
        assert_eq!(config.id, "opaque/config/42");
        assert_eq!(
            agent_text(
                Language::SimplifiedChinese,
                "Custom option from a future agent"
            ),
            "Custom option from a future agent"
        );
    }
}

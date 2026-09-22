//! Design fixtures belong to the presentation layer, never the domain package.
use crate::i18n::{Text, Translate};
use relay_core::{ContextItem, ContextKind, Project, ProjectId, settings::Language};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Inbox,
    Project(usize),
    Agents,
    Settings,
}

/// Reference content for the first workspace preview; never represents live work.
pub fn preview_projects(language: Language) -> Vec<Project> {
    let context = [
        (ContextKind::Web, language.text(Text::ContextClaude)),
        (ContextKind::Web, language.text(Text::ContextDesktop)),
        (ContextKind::Web, language.text(Text::ContextAgent)),
        (ContextKind::Web, language.text(Text::ContextNavigation)),
        (ContextKind::Web, language.text(Text::ContextDesign)),
        (ContextKind::Web, language.text(Text::ContextMemory)),
        (ContextKind::Web, language.text(Text::ContextMultimodal)),
        (ContextKind::Web, language.text(Text::ContextAccessible)),
        (ContextKind::Document, language.text(Text::ContextBrief)),
        (ContextKind::Document, language.text(Text::ContextInterview)),
        (
            ContextKind::Document,
            language.text(Text::ContextPrinciples),
        ),
        (
            ContextKind::Document,
            language.text(Text::ContextArchitecture),
        ),
        (ContextKind::Image, language.text(Text::ContextWorkspace)),
        (ContextKind::Image, language.text(Text::ContextSelection)),
    ]
    .into_iter()
    .map(|(kind, name)| ContextItem {
        name: name.into(),
        kind,
    })
    .collect();

    vec![
        Project {
            id: ProjectId(1),
            name: language.text(Text::ProductDesign).into(),
            description: language.text(Text::ProductDesignDetail).into(),
            context,
        },
        Project {
            id: ProjectId(2),
            name: language.text(Text::AgentInfra).into(),
            description: language.text(Text::AgentInfraDetail).into(),
            context: Vec::new(),
        },
        Project {
            id: ProjectId(3),
            name: language.text(Text::Personal).into(),
            description: language.text(Text::PersonalDetail).into(),
            context: Vec::new(),
        },
    ]
}

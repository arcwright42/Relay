//! Design fixtures belong to the presentation layer, never the domain package.
use relay_core::{ContextItem, ContextKind, Project, ProjectId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Inbox,
    Project(usize),
    Agents,
    Settings,
}

/// Reference content for the first workspace preview; never represents live work.
pub fn preview_projects() -> Vec<Project> {
    let context = [
        (ContextKind::Web, "Claude — Projects redesigned"),
        (ContextKind::Web, "Desktop interaction patterns"),
        (ContextKind::Web, "Agent workspace research"),
        (ContextKind::Web, "Project navigation references"),
        (ContextKind::Web, "Design systems collection"),
        (ContextKind::Web, "Context and memory research"),
        (ContextKind::Web, "Multimodal interaction notes"),
        (ContextKind::Web, "Accessible desktop interfaces"),
        (ContextKind::Document, "Product brief"),
        (ContextKind::Document, "User interview notes"),
        (ContextKind::Document, "Design principles"),
        (ContextKind::Document, "Architecture overview"),
        (ContextKind::Image, "Workspace reference"),
        (ContextKind::Image, "Selection toolbar reference"),
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
            name: "Product Design".into(),
            description: "Turn ideas into reality.".into(),
            context,
        },
        Project {
            id: ProjectId(2),
            name: "Agent Infra".into(),
            description: "Build the foundations for better agents.".into(),
            context: Vec::new(),
        },
        Project {
            id: ProjectId(3),
            name: "Personal".into(),
            description: "A little space for everything else.".into(),
            context: Vec::new(),
        },
    ]
}

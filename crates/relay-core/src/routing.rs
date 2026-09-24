//! Project selection precedes execution and does not belong to an agent harness.
use crate::ProjectId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoutingProvider {
    TypeSafe,
    #[default]
    Vercel,
}

impl RoutingProvider {
    pub fn code(self) -> &'static str {
        match self {
            Self::TypeSafe => "typesafe",
            Self::Vercel => "vercel",
        }
    }
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "typesafe" => Some(Self::TypeSafe),
            "vercel" => Some(Self::Vercel),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::TypeSafe => "TypeSafe",
            Self::Vercel => "Vercel AI Gateway",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteTarget {
    Existing(ProjectId),
    NewProject,
}

#[derive(Clone, Debug)]
pub struct RouteOption {
    pub target: RouteTarget,
    pub probability: f64,
}

#[derive(Clone, Debug)]
pub struct RouteDecision {
    pub catalog_revision: u64,
    /// None means the application must let the user choose before executing.
    pub automatic: Option<RouteTarget>,
    pub options: Vec<RouteOption>,
    pub confidence: f64,
    pub elapsed_ms: u64,
    pub model: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoutingError {
    NotConfigured,
    InvalidKey,
    Keychain,
    Unauthorized,
    RateLimited,
    QuotaExceeded,
    Unavailable,
    InvalidResponse,
    InputTooLarge,
    CatalogUnavailable,
    CatalogChanged,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoutingSnapshot {
    pub provider: RoutingProvider,
    pub configured: bool,
    pub error: Option<RoutingError>,
}

pub trait RoutingService: Send + Sync {
    /// Reads memory only. Credentials are never exposed to the UI.
    fn snapshot(&self) -> RoutingSnapshot;
    /// Blocking I/O: run on a background executor.
    fn save_key(&self, provider: RoutingProvider, key: String) -> Result<(), RoutingError>;
    fn remove_key(&self) -> Result<(), RoutingError>;
    /// Read-only decision. The caller validates the catalog before applying it.
    fn decide(&self, prompt: &str) -> Result<RouteDecision, RoutingError>;
}

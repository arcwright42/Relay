//! Navigation destinations. Project data comes from the project service.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Inbox,
    Sessions,
    Project(usize),
    Agents,
    Settings,
}

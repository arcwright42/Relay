//! Navigation destinations. Thread data comes from the thread service.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Inbox,
    Sessions,
    Files,
    Thread(usize),
    Agents,
    Settings,
}

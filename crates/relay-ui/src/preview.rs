//! Navigation destinations. Thread data comes from the thread service.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Inbox,
    Memory,
    Files,
    Thread(usize),
    Agents,
    Settings,
}

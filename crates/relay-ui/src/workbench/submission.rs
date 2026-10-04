use super::*;
impl Workbench {
    pub(super) fn advance_pending_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, text)) = self.pending_send.clone() else {
            return;
        };
        let Some(index) = self.threads.iter().position(|t| t.id == id) else {
            self.pending_send = None;
            return;
        };
        if self.selected_thread != index {
            self.pending_send = None;
            return;
        }
        let state = &self.agent_states[index];
        if state.status == ConnectionStatus::Failed {
            self.pending_send = None;
            return;
        }
        if state.status != ConnectionStatus::Ready || state.pending_config.is_some() {
            return;
        }
        // The draft may have changed while the connection was opening.
        self.pending_send = None;
        if self.drafts[index].read(cx).value().as_ref() != text {
            return;
        }
        if self.agent_action(AgentCommand::Send(text), cx) {
            self.drafts[index].update(cx, |draft, cx| draft.set_value("", window, cx));
        }
    }
}

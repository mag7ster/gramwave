use crate::{
    tasks::{Operation, RequestId},
    telegram::AuthorizationState,
};
#[derive(Default)]
pub struct AuthCore {
    pub(crate) state: AuthorizationState,
    pub(crate) active: Option<Operation>,
    generation: u64,
}
impl AuthCore {
    pub fn state(&self) -> &AuthorizationState {
        &self.state
    }
    pub fn running(&self) -> bool {
        self.active.is_some()
    }
    pub(crate) fn begin(&mut self) -> Option<Operation> {
        if self.running() {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        let operation = Operation::new(self.generation);
        self.active = Some(operation.clone());
        Some(operation)
    }
    pub(crate) fn accepts(&self, key: RequestId) -> bool {
        self.active.as_ref().is_some_and(|op| op.key() == key)
    }
    pub(crate) fn cancel(&mut self) {
        if let Some(op) = self.active.take() {
            op.cancel.cancel();
        }
    }
    pub(crate) fn transition(&mut self, state: AuthorizationState) -> bool {
        if self.state == state {
            return false;
        }
        self.cancel();
        self.state = state;
        true
    }
}

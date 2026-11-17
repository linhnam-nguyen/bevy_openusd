use std::sync::{Condvar, Mutex};

#[derive(Debug)]
pub(crate) struct LatestMailboxState<T> {
    pending: Option<T>,
    closed: bool,
}

impl<T> Default for LatestMailboxState<T> {
    fn default() -> Self {
        Self {
            pending: None,
            closed: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct LatestMailbox<T> {
    state: Mutex<LatestMailboxState<T>>,
    wake: Condvar,
}

impl<T> LatestMailbox<T> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(LatestMailboxState::default()),
            wake: Condvar::new(),
        }
    }

    pub(crate) fn replace(&self, value: T) -> Result<(), T> {
        let Ok(mut state) = self.state.lock() else {
            return Err(value);
        };
        if state.closed {
            return Err(value);
        }
        state.pending = Some(value);
        self.wake.notify_one();
        Ok(())
    }

    pub(crate) fn pop(&self) -> Option<T> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(value) = state.pending.take() {
                return Some(value);
            }
            if state.closed {
                return None;
            }
            state = self.wake.wait(state).ok()?;
        }
    }

    pub(crate) fn take(&self) -> Option<T> {
        self.state.lock().ok()?.pending.take()
    }

    pub(crate) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.pending = None;
            state.closed = true;
            self.wake.notify_all();
        }
    }
}

//! Cooperative cancellation for engine calls.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A cooperative cancellation signal for a single engine call.
///
/// Cancellation is checked at engine checkpoints during both target preparation
/// and evaluation. Setting it does not abort work in progress; it causes the next
/// checkpoint to return [`ExcelErrorKind::Cancelled`](formualizer_common::ExcelErrorKind).
///
/// The token is a handle: cloning shares one signal, so a caller can hold a clone
/// and cancel from another thread.
///
/// ```
/// use formualizer_eval::engine::CancelToken;
///
/// let token = CancelToken::new();
/// let remote = token.clone();
/// assert!(!token.is_cancelled());
/// remote.cancel();
/// assert!(token.is_cancelled());
/// ```
///
/// The underlying representation is deliberately private so that richer
/// cancellation (cancellation reasons, linked child tokens) can be added without
/// a breaking change. Use [`CancelToken::from_flag`] to adopt a flag you already
/// own.
#[derive(Clone)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
    checkpoint: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelToken")
            .field("flag", &self.flag)
            .field("has_checkpoint", &self.checkpoint.is_some())
            .finish()
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelToken {
    /// Creates a token that has not been cancelled.
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            checkpoint: None,
        }
    }

    /// Signals cancellation. Idempotent, and safe to call from another thread.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// Returns whether cancellation has been signalled.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        if self.flag.load(Ordering::Relaxed) {
            return true;
        }
        if self
            .checkpoint
            .as_ref()
            .is_some_and(|checkpoint| checkpoint())
        {
            self.cancel();
            return true;
        }
        false
    }

    /// Adds the caller's synchronous stop condition. Clones share the
    /// condition; once it asks to stop, the signal stays cancelled. The
    /// condition must be thread-safe and must not call back into the engine.
    #[must_use]
    pub fn with_checkpoint(
        mut self,
        checkpoint: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        self.checkpoint = Some(Arc::new(checkpoint));
        self
    }

    /// Adopts an existing flag, for callers that already own one or share it with
    /// non-engine code.
    pub fn from_flag(flag: Arc<AtomicBool>) -> Self {
        Self {
            flag,
            checkpoint: None,
        }
    }

    /// Borrows the underlying flag as a legacy/interoperability bridge.
    ///
    /// New code should poll [`CancelToken::is_cancelled`] instead. The raw flag
    /// may not represent future richer cancellation semantics.
    pub fn as_flag(&self) -> &Arc<AtomicBool> {
        &self.flag
    }
}

impl From<Arc<AtomicBool>> for CancelToken {
    fn from(flag: Arc<AtomicBool>) -> Self {
        Self::from_flag(flag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_one_signal() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!token.is_cancelled());
        clone.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn adopting_a_flag_observes_external_cancellation() {
        let flag = Arc::new(AtomicBool::new(false));
        let token = CancelToken::from_flag(Arc::clone(&flag));
        assert!(!token.is_cancelled());
        flag.store(true, Ordering::Relaxed);
        assert!(token.is_cancelled());
    }

    #[test]
    fn default_token_is_not_cancelled() {
        assert!(!CancelToken::default().is_cancelled());
    }
}

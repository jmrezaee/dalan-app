//! Quitting with live sessions: closing the window or pressing ⌘Q first asks, because quitting ends every shell.
//! `run()` in lib.rs asks this guard what to do; the UI shows the question and answers with `quit_app` or `quit_cancel`.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to lose (or the user insisted): let the app close.
    Allow,
    /// Hold the close back and ask the UI, which has this many live sessions to lose.
    Ask(u32),
}

#[derive(Default)]
pub struct QuitGuard {
    /// True while the question is on screen.
    asked: AtomicBool,
}

impl QuitGuard {
    pub fn decide(&self, live_sessions: usize) -> Decision {
        if live_sessions == 0 {
            self.asked.store(false, Ordering::SeqCst);
            return Decision::Allow;
        }
        // A second quit while the question is still open (⌘Q twice, or a window that cannot show it) goes through,
        // so a broken page can never trap the user in a running app.
        if self.asked.swap(true, Ordering::SeqCst) {
            return Decision::Allow;
        }
        Decision::Ask(u32::try_from(live_sessions).unwrap_or(u32::MAX))
    }

    /// The user answered "Keep working": the next quit asks again.
    pub fn cancel(&self) {
        self.asked.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_sessions_quits_at_once() {
        assert_eq!(QuitGuard::default().decide(0), Decision::Allow);
    }

    #[test]
    fn live_sessions_ask_first() {
        assert_eq!(QuitGuard::default().decide(3), Decision::Ask(3));
    }

    #[test]
    fn a_second_quit_while_the_question_is_open_goes_through() {
        let g = QuitGuard::default();
        assert_eq!(g.decide(2), Decision::Ask(2));
        assert_eq!(g.decide(2), Decision::Allow);
    }

    #[test]
    fn keep_working_makes_the_next_quit_ask_again() {
        let g = QuitGuard::default();
        assert_eq!(g.decide(1), Decision::Ask(1));
        g.cancel();
        assert_eq!(g.decide(1), Decision::Ask(1));
    }
}

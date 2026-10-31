//! Questions waiting for the user (permissions and plans). A turn registers a request and waits;
//! whichever of the person's clients answers first wins (protocol §9). Late answers are refused,
//! and so are answers from anyone but the person whose session asked.

use std::collections::HashMap;
use std::sync::Mutex;

use thrall_protocol::PermissionChoice;
use tokio::sync::oneshot;

/// What the user decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// A permission answer.
    Permission(PermissionChoice),
    /// A plan decision.
    Plan {
        /// Go ahead with the plan.
        approve: bool,
        /// And accept edits without asking from now on.
        auto_edits: bool,
    },
}

/// An answer and who gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The decision.
    pub reply: Reply,
    /// Which client answered (`tui`, `terminal`, `web`…).
    pub answered_by: String,
}

/// A request waiting: whose it is, and where the answer goes.
struct Pending {
    user: String,
    answer: oneshot::Sender<Answer>,
}

/// Pending requests, by id.
#[derive(Default)]
pub struct Prompts {
    pending: Mutex<HashMap<String, Pending>>,
}

impl Prompts {
    /// Register a request asked of `user` (the session's person); the returned receiver gets
    /// the answer.
    pub fn register(&self, request_id: &str, user: &str) -> oneshot::Receiver<Answer> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(
                request_id.to_owned(),
                Pending {
                    user: user.to_owned(),
                    answer: tx,
                },
            );
        }
        rx
    }

    /// Answer a request as `user`. False if it was already answered, never existed, or isn't
    /// theirs to answer (it stays waiting for its own person).
    pub fn answer(&self, request_id: &str, user: &str, reply: Reply, answered_by: &str) -> bool {
        let sender =
            self.pending
                .lock()
                .ok()
                .and_then(|mut pending| match pending.get(request_id) {
                    Some(p) if p.user == user => pending.remove(request_id),
                    _ => None,
                });
        match sender {
            Some(p) => p
                .answer
                .send(Answer {
                    reply,
                    answered_by: answered_by.to_owned(),
                })
                .is_ok(),
            None => false,
        }
    }

    /// Forget a request nobody will answer (the turn ended or was interrupted).
    pub fn cancel(&self, request_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(request_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_answer_wins() {
        let prompts = Prompts::default();
        let rx = prompts.register("r1", "u_me");
        // Someone else can't answer it, and their try doesn't use it up.
        assert!(!prompts.answer(
            "r1",
            "u_other",
            Reply::Permission(PermissionChoice::No),
            "web"
        ));
        assert!(prompts.answer(
            "r1",
            "u_me",
            Reply::Permission(PermissionChoice::Yes),
            "tui"
        ));
        assert!(!prompts.answer("r1", "u_me", Reply::Permission(PermissionChoice::No), "web"));
        let answer = rx.await.unwrap();
        assert_eq!(answer.reply, Reply::Permission(PermissionChoice::Yes));
        assert_eq!(answer.answered_by, "tui");
    }

    #[test]
    fn unknown_requests_are_refused() {
        assert!(!Prompts::default().answer(
            "nope",
            "u_me",
            Reply::Permission(PermissionChoice::Yes),
            "tui"
        ));
    }
}

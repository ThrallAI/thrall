//! Questions waiting for the user (permissions, plans and question cards). A turn registers a request and waits;
//! whichever of the person's clients answers first wins (protocol §9). Late answers are refused,
//! and so are answers from anyone but the person whose session asked.

use std::collections::HashMap;
use std::sync::Mutex;

use thrall_protocol::PermissionChoice;
use tokio::sync::oneshot;

use crate::constants::FULL_AUTO_ANSWERER;

/// What the user decided.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Answers to a question card, one per question.
    Question(Vec<String>),
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
    /// The session of a permission card that Full auto wouldn't have asked about (it's
    /// answered yes when Full auto goes on there).
    full_auto_session: Option<String>,
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
        self.register_card(request_id, user, None)
    }

    /// Register a permission card; `full_auto_session` is its session when Full auto wouldn't
    /// have asked, so switching Full auto on there answers it.
    pub fn register_card(
        &self,
        request_id: &str,
        user: &str,
        full_auto_session: Option<&str>,
    ) -> oneshot::Receiver<Answer> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(
                request_id.to_owned(),
                Pending {
                    user: user.to_owned(),
                    full_auto_session: full_auto_session.map(str::to_owned),
                    answer: tx,
                },
            );
        }
        rx
    }

    /// Full auto went on in `session_id`: answer yes to `user`'s cards there that it wouldn't
    /// have asked about. Cards it still asks about (High risk) stay open. How many were answered.
    pub fn answer_for_full_auto(&self, session_id: &str, user: &str) -> usize {
        let Ok(mut pending) = self.pending.lock() else {
            return 0;
        };
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, p)| p.user == user && p.full_auto_session.as_deref() == Some(session_id))
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter()
            .filter_map(|id| pending.remove(id))
            .map(|p| {
                p.answer
                    .send(Answer {
                        reply: Reply::Permission(PermissionChoice::Yes),
                        answered_by: FULL_AUTO_ANSWERER.to_owned(),
                    })
                    .is_ok()
            })
            .filter(|sent| *sent)
            .count()
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

    #[tokio::test]
    async fn full_auto_answers_only_the_cards_it_would_not_ask() {
        let prompts = Prompts::default();
        let medium = prompts.register_card("r1", "u_me", Some("s1"));
        let _high = prompts.register_card("r2", "u_me", None);
        let _elsewhere = prompts.register_card("r3", "u_me", Some("s2"));
        // Someone else's Full auto answers nothing of mine.
        assert_eq!(prompts.answer_for_full_auto("s1", "u_other"), 0);
        assert_eq!(prompts.answer_for_full_auto("s1", "u_me"), 1);
        let answer = medium.await.unwrap();
        assert_eq!(answer.reply, Reply::Permission(PermissionChoice::Yes));
        assert_eq!(answer.answered_by, FULL_AUTO_ANSWERER);
        // The High-risk card and the other chat's card still wait for the person.
        assert!(prompts.answer("r2", "u_me", Reply::Permission(PermissionChoice::No), "web"));
        assert!(prompts.answer("r3", "u_me", Reply::Permission(PermissionChoice::No), "web"));
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

//! A companion PC's runner on its link: calls for the PC go out as `runner.call` events, and the
//! runner's `runner.output` and `runner.done` requests answer them.

use std::collections::HashMap;

use serde_json::Value;
use thrall_agent::companions::Outgoing;
use thrall_core::ids::now_ms;
use thrall_protocol::{Event, EventFrame, ServerFrame};
use tokio::sync::{mpsc, oneshot};

/// Answers waiting for a runner, by call id.
type Waiting = HashMap<
    String,
    (
        oneshot::Sender<Result<Value, String>>,
        Option<mpsc::UnboundedSender<String>>,
    ),
>;

/// The runner side of one PC's link.
pub(crate) struct RunnerSide {
    /// The PC (a joined device's id).
    pub device: String,
    /// Its registration, for `detach`.
    pub token: u64,
    waiting: Waiting,
}

impl RunnerSide {
    pub fn new(device: String, token: u64) -> Self {
        Self {
            device,
            token,
            waiting: HashMap::new(),
        }
    }

    /// A call for the runner, as the event that carries it.
    pub fn send(&mut self, out: Outgoing) -> Option<ServerFrame> {
        let job = match serde_json::to_value(&out.call) {
            Ok(job) => job,
            Err(e) => {
                let _ = out.reply.send(Err(e.to_string()));
                return None;
            }
        };
        self.waiting.insert(out.id.clone(), (out.reply, out.output));
        Some(ServerFrame::Event(EventFrame::new(
            "",
            None,
            now_ms(),
            Event::RunnerCall {
                call: out.id,
                person: out.person,
                in_session: out.session,
                workspace: out.workspace,
                job,
            },
        )))
    }

    /// Live output of a call.
    pub fn output(&self, call: &str, text: String) {
        if let Some((_, Some(output))) = self.waiting.get(call) {
            let _ = output.send(text);
        }
    }

    /// A call finished.
    pub fn done(&mut self, call: &str, result: Option<Value>, error: Option<String>) {
        if let Some((reply, _)) = self.waiting.remove(call) {
            let _ = reply.send(match error {
                Some(why) => Err(why),
                None => Ok(result.unwrap_or(Value::Null)),
            });
        }
    }
}

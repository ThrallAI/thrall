//! What the page sends and sees: runs, their lanes, and each lane's live numbers.

use serde::{Deserialize, Serialize};

/// How a lane runs Thrall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneKind {
    /// One model does everything.
    Single,
    /// Split mode: a Thinker plans and checks, a Doer does the work.
    Split,
}

/// One lane's setup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneSpec {
    /// Shown on the lane's card.
    #[serde(default)]
    pub label: String,
    /// Single model or Thinker + Doer.
    pub kind: LaneKind,
    /// The model (single).
    #[serde(default)]
    pub model: String,
    /// The Thinker's model (split).
    #[serde(default)]
    pub thinker: String,
    /// The Doer's model (split).
    #[serde(default)]
    pub doer: String,
}

impl LaneSpec {
    /// A name for the lane: its label, or its models.
    pub fn name(&self) -> String {
        if !self.label.trim().is_empty() {
            return self.label.trim().to_owned();
        }
        match self.kind {
            LaneKind::Single => self.model.clone(),
            LaneKind::Split => format!("{} + {}", self.thinker, self.doer),
        }
    }

    /// Why this setup can't run, if it can't.
    pub fn problem(&self) -> Option<String> {
        let missing = match self.kind {
            LaneKind::Single => self.model.trim().is_empty(),
            LaneKind::Split => self.thinker.trim().is_empty() || self.doer.trim().is_empty(),
        };
        missing.then(|| format!("lane \"{}\" needs its model(s)", self.name()))
    }
}

/// Start a run.
#[derive(Debug, Clone, Deserialize)]
pub struct RunRequest {
    /// What every lane is asked.
    pub prompt: String,
    /// Task folder name (copied into each lane; its check scores the result). None: an empty
    /// folder and no check.
    #[serde(default)]
    pub task: Option<String>,
    /// Most each lane may spend, in US dollars; the lane is stopped when it passes this.
    #[serde(default)]
    pub budget_usd: Option<f64>,
    /// The lanes.
    pub lanes: Vec<LaneSpec>,
}

/// A run, as the page shows it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunView {
    /// Run id.
    pub id: String,
    /// When it started, Unix ms.
    pub started_at: i64,
    /// The prompt.
    pub prompt: String,
    /// The task folder, if any.
    pub task: Option<String>,
    /// Budget per lane.
    pub budget_usd: f64,
    /// Every lane finished (or failed).
    pub finished: bool,
    /// The lanes.
    pub lanes: Vec<LaneView>,
}

/// One lane's live numbers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaneView {
    /// Its setup.
    pub spec: Option<LaneSpec>,
    /// preparing, ready, running, checking, done, over budget, stopped, failed.
    pub status: String,
    /// Time since the lanes started (frozen when it finished).
    pub elapsed_ms: u64,
    /// Model calls (every usage row: Thinker, Doer, questions, the title).
    pub model_calls: u64,
    /// Tool steps shown.
    pub steps: u64,
    /// Steps refused (the Thinker trying to change something, or "always ask" actions).
    pub denied: u64,
    /// Hand-offs to the Doer.
    pub hand_offs: u64,
    /// Questions the Doer asked the Thinker.
    pub asks: u64,
    /// Tokens sent.
    pub input_tokens: u64,
    /// Of those, cached.
    pub cached_tokens: u64,
    /// Tokens written.
    pub output_tokens: u64,
    /// Dollars spent.
    pub cost_usd: f64,
    /// The same, per model and role.
    pub by_model: Vec<ModelCost>,
    /// What happened, newest last.
    pub feed: Vec<String>,
    /// Thrall's final reply.
    pub reply: String,
    /// The task's check, once run.
    pub check: Option<CheckResult>,
    /// What went wrong, if the lane failed.
    pub error: Option<String>,
}

/// One model's share of a lane.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    /// Model id.
    pub model: String,
    /// chat, thinker, doer or ask.
    pub role: String,
    /// Calls.
    pub calls: u64,
    /// Tokens sent.
    pub input_tokens: u64,
    /// Of those, cached.
    pub cached_tokens: u64,
    /// Tokens written.
    pub output_tokens: u64,
    /// Dollars.
    pub cost_usd: f64,
}

/// How a task's check went.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckResult {
    /// It exited with success.
    pub passed: bool,
    /// Tests passed and total, when it printed `SCORE passed/total`.
    pub score: Option<(u32, u32)>,
    /// The end of what it printed.
    pub output: String,
}

/// A test task, from its `task.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Folder name.
    #[serde(default)]
    pub name: String,
    /// Shown in the list.
    #[serde(default)]
    pub title: String,
    /// What the lanes are asked.
    pub prompt: String,
    /// Run in the lane's folder after its turn; success = passed. Empty: no check.
    #[serde(default)]
    pub check: String,
}

/// A saved lane setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    /// Its name.
    pub name: String,
    /// The lanes.
    pub lanes: Vec<LaneSpec>,
    /// Budget per lane.
    #[serde(default)]
    pub budget_usd: Option<f64>,
}

/// A model on offer, with its prices per million tokens.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    /// Model id.
    pub id: String,
    /// Dollars per million tokens sent.
    pub prompt_per_m: f64,
    /// Dollars per million tokens written.
    pub completion_per_m: f64,
}

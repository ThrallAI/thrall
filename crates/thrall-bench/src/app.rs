//! The bench: runs (live in memory, saved when finished), presets, tasks, and the model list.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};
use thrall_agent::AppState;
use thrall_core::Config;
use thrall_core::ids::now_ms;

use crate::constants::{
    DEFAULT_BUDGET_USD, MAX_LANES, PRESETS_FILE, RESULT_FILE, RUNS_DIR, TASK_FILE,
};
use crate::lane::{self, status};
use crate::model::{LaneView, ModelInfo, Preset, RunRequest, RunView, Task};

/// Everything the bench keeps.
pub struct Bench {
    /// The bench's data folder (runs, presets).
    pub dir: PathBuf,
    /// Where the test tasks are.
    pub tasks_dir: PathBuf,
    /// The person's Thrall settings (each lane takes its provider and checks from them).
    pub base: Config,
    /// The OpenRouter key.
    pub api_key: String,
    runs: Mutex<HashMap<String, RunView>>,
    /// Running lanes' engines and sessions, to stop them.
    running: Mutex<HashMap<String, Vec<(AppState, String)>>>,
    models: tokio::sync::Mutex<Option<Vec<ModelInfo>>>,
    http: reqwest::Client,
}

impl Bench {
    /// A bench keeping its data in `dir`.
    pub fn new(dir: PathBuf, tasks_dir: PathBuf, base: Config, api_key: String) -> Self {
        Self {
            dir,
            tasks_dir,
            base,
            api_key,
            runs: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            models: tokio::sync::Mutex::new(None),
            http: reqwest::Client::new(),
        }
    }

    /// Change one lane's view.
    pub fn update(&self, run_id: &str, index: usize, f: &mut dyn FnMut(&mut LaneView)) {
        if let Ok(mut runs) = self.runs.lock()
            && let Some(lane) = runs.get_mut(run_id).and_then(|r| r.lanes.get_mut(index))
        {
            f(lane);
        }
    }

    /// One lane's view, as it is now.
    pub fn lane(&self, run_id: &str, index: usize) -> Option<LaneView> {
        let runs = self.runs.lock().ok()?;
        runs.get(run_id)?.lanes.get(index).cloned()
    }

    /// Start a run; its id, or why it can't start.
    pub fn start(self: &Arc<Self>, request: RunRequest) -> Result<String, String> {
        if request.prompt.trim().is_empty() {
            return Err("write the prompt first".to_owned());
        }
        if request.lanes.is_empty() || request.lanes.len() > MAX_LANES {
            return Err(format!("use 1 to {MAX_LANES} lanes"));
        }
        if let Some(problem) = request.lanes.iter().find_map(|l| l.problem()) {
            return Err(problem);
        }
        let task = match request.task.as_deref().filter(|t| !t.is_empty()) {
            Some(name) => Some(self.task(name)?),
            None => None,
        };
        let budget_usd = request
            .budget_usd
            .filter(|b| *b > 0.0)
            .unwrap_or(DEFAULT_BUDGET_USD);
        let id = format!("r{}", now_ms());
        let view = RunView {
            id: id.clone(),
            started_at: now_ms(),
            prompt: request.prompt.clone(),
            task: task.as_ref().map(|t| t.name.clone()),
            budget_usd,
            finished: false,
            lanes: request
                .lanes
                .iter()
                .map(|spec| LaneView {
                    spec: Some(spec.clone()),
                    status: status::PREPARING.to_owned(),
                    ..LaneView::default()
                })
                .collect(),
        };
        self.runs
            .lock()
            .map_err(|_| "the bench is broken; restart it".to_owned())?
            .insert(id.clone(), view);
        let bench = self.clone();
        let run_id = id.clone();
        tokio::spawn(async move {
            bench.execute(&run_id, request, task, budget_usd).await;
        });
        Ok(id)
    }

    /// Prepare every lane, start them at the same moment, wait for all, save the results.
    async fn execute(&self, run_id: &str, request: RunRequest, task: Option<Task>, budget: f64) {
        let run_dir = self.dir.join(RUNS_DIR).join(run_id);
        let prepared =
            futures::future::join_all(request.lanes.iter().enumerate().map(|(index, spec)| {
                let lane_dir = run_dir.join(format!("lane{}", index + 1));
                let task = task.as_ref();
                async move { lane::prepare(self, &lane_dir, spec, task).await }
            }))
            .await;
        let mut lanes = Vec::new();
        for (index, result) in prepared.into_iter().enumerate() {
            match result {
                Ok(lane) => lanes.push((index, lane)),
                Err(e) => self.update(run_id, index, &mut |v| {
                    v.status = status::FAILED.to_owned();
                    v.error = Some(format!("couldn't set the lane up: {e}"));
                }),
            }
        }
        if let Ok(mut running) = self.running.lock() {
            running.insert(
                run_id.to_owned(),
                lanes
                    .iter()
                    .map(|(_, l)| (l.state.clone(), l.session_id.clone()))
                    .collect(),
            );
        }
        let start = Instant::now();
        let prompt = request.prompt.as_str();
        futures::future::join_all(lanes.iter().map(|(index, lane)| {
            lane::run(
                self,
                run_id,
                *index,
                lane,
                prompt,
                budget,
                start,
                task.as_ref(),
            )
        }))
        .await;
        if let Ok(mut running) = self.running.lock() {
            running.remove(run_id);
        }
        let finished = self.runs.lock().ok().and_then(|mut runs| {
            let run = runs.get_mut(run_id)?;
            run.finished = true;
            Some(run.clone())
        });
        if let Some(run) = finished {
            let saved = serde_json::to_string_pretty(&run)
                .map_err(std::io::Error::other)
                .and_then(|text| std::fs::write(run_dir.join(RESULT_FILE), text));
            if let Err(e) = saved {
                tracing::warn!(%e, run = run_id, "couldn't save the run's results");
            }
        }
    }

    /// Stop a run's lanes.
    pub fn stop(&self, run_id: &str) -> bool {
        let lanes = self
            .running
            .lock()
            .ok()
            .and_then(|r| r.get(run_id).cloned())
            .unwrap_or_default();
        for (state, session_id) in &lanes {
            state.interrupt(session_id);
        }
        if let Ok(mut runs) = self.runs.lock()
            && let Some(run) = runs.get_mut(run_id)
        {
            for lane in &mut run.lanes {
                if lane.status == status::RUNNING {
                    status::STOPPING.clone_into(&mut lane.status);
                }
            }
        }
        !lanes.is_empty()
    }

    /// A run: live, or saved.
    pub fn run(&self, run_id: &str) -> Option<RunView> {
        if let Some(run) = self.runs.lock().ok()?.get(run_id) {
            return Some(run.clone());
        }
        if !plain_name(run_id) {
            return None;
        }
        let text =
            std::fs::read_to_string(self.dir.join(RUNS_DIR).join(run_id).join(RESULT_FILE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Every run, newest first, briefly (for the history list).
    pub fn history(&self) -> Vec<Value> {
        let mut ids: Vec<String> = std::fs::read_dir(self.dir.join(RUNS_DIR))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        if let Ok(runs) = self.runs.lock() {
            ids.extend(runs.keys().cloned());
        }
        ids.sort();
        ids.dedup();
        ids.reverse();
        ids.iter()
            .filter_map(|id| self.run(id))
            .map(|run| {
                json!({
                    "id": run.id,
                    "started_at": run.started_at,
                    "prompt": run.prompt.chars().take(120).collect::<String>(),
                    "task": run.task,
                    "finished": run.finished,
                    "lanes": run.lanes.iter().map(|l| json!({
                        "name": l.spec.as_ref().map(crate::model::LaneSpec::name),
                        "status": l.status,
                        "cost_usd": l.cost_usd,
                        "elapsed_ms": l.elapsed_ms,
                        "score": l.check.as_ref().and_then(|c| c.score),
                        "passed": l.check.as_ref().map(|c| c.passed),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect()
    }

    /// The test tasks.
    pub fn tasks(&self) -> Vec<Task> {
        let mut tasks: Vec<Task> = std::fs::read_dir(&self.tasks_dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|e| self.task(&e.file_name().to_string_lossy()).ok())
                    .collect()
            })
            .unwrap_or_default();
        tasks.sort_by(|a, b| a.name.cmp(&b.name));
        tasks
    }

    /// One task, from its folder's `task.toml`.
    pub fn task(&self, name: &str) -> Result<Task, String> {
        if !plain_name(name) {
            return Err(format!("no task called {name}"));
        }
        let path = self.tasks_dir.join(name).join(TASK_FILE);
        let text = std::fs::read_to_string(&path).map_err(|_| format!("no task called {name}"))?;
        let mut task: Task =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        name.clone_into(&mut task.name);
        if task.title.trim().is_empty() {
            name.clone_into(&mut task.title);
        }
        Ok(task)
    }

    /// Saved lane setups.
    pub fn presets(&self) -> Vec<Preset> {
        std::fs::read_to_string(self.dir.join(PRESETS_FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Save a lane setup (replacing one with the same name).
    pub fn save_preset(&self, preset: Preset) -> Result<(), String> {
        if preset.name.trim().is_empty() {
            return Err("give the preset a name".to_owned());
        }
        let mut presets = self.presets();
        presets.retain(|p| p.name != preset.name);
        presets.push(preset);
        presets.sort_by(|a, b| a.name.cmp(&b.name));
        let text = serde_json::to_string_pretty(&presets).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join(PRESETS_FILE), text).map_err(|e| e.to_string())
    }

    /// OpenRouter's models with their prices (fetched once).
    pub async fn models(&self) -> Result<Vec<ModelInfo>, String> {
        let mut cached = self.models.lock().await;
        if let Some(models) = cached.as_ref() {
            return Ok(models.clone());
        }
        let url = format!(
            "{}/models",
            self.base.provider.base_url.trim_end_matches('/')
        );
        let body: Value = self
            .http
            .get(url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let per_million = |v: Option<&Value>| {
            v.and_then(Value::as_str)
                .and_then(|s| s.parse::<f64>().ok())
                .map_or(0.0, |p| p * 1_000_000.0)
        };
        let mut models: Vec<ModelInfo> = body
            .get("data")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|m| {
                        Some(ModelInfo {
                            id: m.get("id")?.as_str()?.to_owned(),
                            prompt_per_m: per_million(m.pointer("/pricing/prompt")),
                            completion_per_m: per_million(m.pointer("/pricing/completion")),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        *cached = Some(models.clone());
        Ok(models)
    }
}

/// A folder name with nothing that could lead elsewhere.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && Path::new(name).components().count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_names_reach_the_disk() {
        assert!(plain_name("r1730000000000"));
        assert!(plain_name("fix-the-parser_2"));
        assert!(!plain_name("../secrets"));
        assert!(!plain_name("a/b"));
        assert!(!plain_name("C:"));
        assert!(!plain_name(""));
    }
}

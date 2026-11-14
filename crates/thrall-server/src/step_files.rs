//! Files a step wrote (`step.completed`'s `saved`), for the desktop app to open and for phones and
//! browsers to fetch. A file is found from the session's own stored events by its step and its
//! place in that list, never from a path a client sends, so only files Thrall wrote in that
//! session can be reached.

use std::path::PathBuf;

use thrall_agent::AppState;
use thrall_protocol::Event;
use thrall_protocol::constants::FILE_CHUNK_BYTES;

use crate::constants::{REPLAY_PAGE, STEP_FILE_GONE, STEP_FILE_MAX_BYTES, STEP_FILE_TOO_BIG};

/// The file at `index` in `step`'s `saved` list, if the step is in `session` and the file is still
/// there.
pub async fn find(
    state: &AppState,
    session: &str,
    step: &str,
    index: u32,
) -> Result<PathBuf, String> {
    let mut after = 0;
    loop {
        let page = state
            .store
            .events_after(&state.user_id, session, after, REPLAY_PAGE)
            .await
            .map_err(|e| format!("Couldn't look the step up: {e}"))?;
        let count = page.len();
        for stored in page {
            after = stored.seq;
            // Most events aren't this step's: skip them without reading them.
            if !stored.payload.contains(step) {
                continue;
            }
            if let Ok(Event::StepCompleted {
                step: done, saved, ..
            }) = serde_json::from_str::<Event>(&stored.payload)
                && done == step
            {
                let file = saved
                    .get(index as usize)
                    .map(|f| PathBuf::from(&f.path))
                    .ok_or_else(|| "That step wrote no such file.".to_owned())?;
                return if file.is_file() {
                    Ok(file)
                } else {
                    Err(STEP_FILE_GONE.to_owned())
                };
            }
        }
        if count < REPLAY_PAGE as usize {
            return Err("There's no such step in this chat.".to_owned());
        }
    }
}

/// Up to `FILE_CHUNK_BYTES` of `file` from `offset`: (its name, where the piece starts, the whole
/// size, the bytes). Files over `STEP_FILE_MAX_BYTES` aren't sent.
pub async fn read(file: &PathBuf, offset: u64) -> Result<(String, u64, u64, Vec<u8>), String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let gone = || STEP_FILE_GONE.to_owned();
    let mut f = tokio::fs::File::open(file).await.map_err(|_| gone())?;
    let size = f.metadata().await.map_err(|_| gone())?.len();
    if size > STEP_FILE_MAX_BYTES {
        return Err(STEP_FILE_TOO_BIG.to_owned());
    }
    let start = offset.min(size);
    f.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| gone())?;
    let want = (size - start).min(FILE_CHUNK_BYTES as u64) as usize;
    let mut buf = vec![0u8; want];
    f.read_exact(&mut buf).await.map_err(|_| gone())?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((name, start, size, buf))
}

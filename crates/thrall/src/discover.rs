//! Finding a running engine: `engine.json` says where it listens, and its health check proves
//! it's really there (a crashed engine can leave the file behind).

use thrall_core::Paths;
use thrall_server::EngineFile;
use thrall_server::constants::HEALTH_PATH;

use crate::constants::HEALTH_TIMEOUT;

/// The engine that's running now, if any.
pub async fn running(paths: &Paths) -> Option<EngineFile> {
    let file = EngineFile::read(&paths.engine_file)?;
    let url = format!("{}{}", file.web_url.trim_end_matches('/'), HEALTH_PATH);
    let client = reqwest::Client::builder()
        .timeout(HEALTH_TIMEOUT)
        .no_proxy()
        .build()
        .ok()?;
    let ok = client
        .get(url)
        .send()
        .await
        .is_ok_and(|r| r.status().is_success());
    ok.then_some(file)
}

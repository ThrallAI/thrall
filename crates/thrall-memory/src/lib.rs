//! Long-term memory: facts (semantic, superseded rather than overwritten), past conversations
//! (episodic), search by words and by meaning, and context compaction planning.
//!
//! Search is hybrid: SQLite full-text search finds word matches, embeddings find matches by
//! meaning, and the two rankings are merged (reciprocal rank fusion). Without an embedder (no
//! key, or offline), word search still works.

pub mod compaction;
pub mod constants;
pub mod trim;

use std::collections::HashMap;
use std::sync::Arc;

use thrall_core::constants::{id_prefix, usage_role};
use thrall_core::ids::{new_id, now_ms};
use thrall_llm::Embedder;
use thrall_store::{Episode, MemoryKind, Store, StoreError, UsageRecord};

use crate::constants::{
    CANDIDATES_PER_SOURCE, EMBED_MAX_CHARS, EPISODE_MAX_CHARS, MIN_SIMILARITY, RRF_K,
    TOPIC_FILLER_WORDS,
};

/// One thing recalled from memory.
#[derive(Debug, Clone, PartialEq)]
pub struct Recall {
    /// Fact or past conversation.
    pub kind: MemoryKind,
    /// Its id.
    pub id: String,
    /// Topic (fact) or summary (conversation).
    pub title: String,
    /// The fact, or what was said.
    pub text: String,
    /// When, Unix ms.
    pub at: i64,
    /// How well it matched (higher is better; only for ordering).
    pub score: f32,
}

/// A saved fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFact {
    /// The new fact's id.
    pub id: String,
    /// Facts on the same topic it replaced.
    pub replaced: Vec<String>,
}

/// The memory service.
pub struct Memory {
    store: Arc<dyn Store>,
    embedder: Option<Arc<dyn Embedder>>,
    model: String,
    provider: String,
}

impl Memory {
    /// Memory over `store`; with an embedder, searches by meaning too.
    pub fn new(
        store: Arc<dyn Store>,
        embedder: Option<Arc<dyn Embedder>>,
        model: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        Self {
            store,
            embedder,
            model: model.into(),
            provider: provider.into(),
        }
    }

    /// Whether searches by meaning are available.
    pub fn has_embeddings(&self) -> bool {
        self.embedder.is_some()
    }

    /// Vectors for `texts`, or `None` when there's no embedder or the call failed (search by
    /// words still works). The cost is recorded like any model call.
    async fn embed(&self, user_id: &str, texts: Vec<String>) -> Option<Vec<Vec<f32>>> {
        let embedder = self.embedder.as_ref()?;
        let texts: Vec<String> = texts
            .into_iter()
            .map(|t| t.chars().take(EMBED_MAX_CHARS).collect())
            .collect();
        match embedder.embed(&self.model, texts).await {
            Ok(e) => {
                let record = UsageRecord {
                    id: new_id(id_prefix::USAGE),
                    user_id: user_id.to_owned(),
                    session_id: None,
                    turn_id: None,
                    provider: self.provider.clone(),
                    model: self.model.clone(),
                    role: usage_role::CHAT.to_owned(),
                    input_tokens: e.tokens,
                    cached_tokens: 0,
                    output_tokens: 0,
                    cost_usd: e.cost_usd,
                    at: now_ms(),
                };
                if let Err(err) = self.store.record_usage(record).await {
                    tracing::warn!(%err, "couldn't record embedding usage");
                }
                Some(e.vectors)
            }
            Err(e) => {
                tracing::warn!(%e, "embedding failed; memory search falls back to words");
                None
            }
        }
    }

    async fn embed_and_store(&self, user_id: &str, kind: MemoryKind, id: &str, text: String) {
        if let Some(mut vectors) = self.embed(user_id, vec![text]).await
            && let Some(vector) = vectors.pop()
            && let Err(e) = self
                .store
                .put_embedding(user_id, kind, id, &self.model, &vector)
                .await
        {
            tracing::warn!(%e, "couldn't store an embedding");
        }
    }

    /// Save a fact. Facts already saved under the same topic are superseded (kept for history,
    /// no longer recalled).
    pub async fn save_fact(
        &self,
        user_id: &str,
        topic: &str,
        content: &str,
    ) -> Result<SavedFact, StoreError> {
        self.save_fact_replacing(user_id, topic, content, &[]).await
    }

    /// Save a fact. It replaces current facts on the same topic (ignoring filler words such as
    /// "updated" or "new") and any listed in `replaces`. A replacement keeps the earlier topic's
    /// wording, so one subject stays under one topic.
    pub async fn save_fact_replacing(
        &self,
        user_id: &str,
        topic: &str,
        content: &str,
        replaces: &[String],
    ) -> Result<SavedFact, StoreError> {
        let id = new_id(id_prefix::KNOWLEDGE);
        let key = topic_key(topic);
        let mut earlier = self.store.knowledge_by_topic(user_id, topic).await?;
        // Same topic in other words: candidates from word search, compared by topic key.
        let hits = self
            .store
            .search_memory_text(user_id, topic, CANDIDATES_PER_SOURCE)
            .await?;
        let mut ids: Vec<String> = hits
            .into_iter()
            .filter(|h| h.kind == MemoryKind::Knowledge)
            .map(|h| h.id)
            .collect();
        ids.extend(replaces.iter().cloned());
        for k in self.store.get_knowledge(user_id, &ids).await? {
            let same = !key.is_empty() && topic_key(&k.topic) == key;
            if (same || replaces.contains(&k.id)) && !earlier.iter().any(|e| e.id == k.id) {
                earlier.push(k);
            }
        }
        let topic = earlier
            .iter()
            .find(|e| topic_key(&e.topic) == key)
            .map_or_else(|| topic.trim().to_owned(), |e| e.topic.clone());
        let at = now_ms();
        self.store
            .save_knowledge(user_id, &id, &topic, content.trim(), at)
            .await?;
        let mut replaced = Vec::new();
        for old in earlier {
            self.store
                .supersede_knowledge(user_id, &old.id, &id, at)
                .await?;
            replaced.push(old.id);
        }
        self.embed_and_store(
            user_id,
            MemoryKind::Knowledge,
            &id,
            format!("{topic}: {content}"),
        )
        .await;
        Ok(SavedFact { id, replaced })
    }

    /// Remember a conversation (one turn: what was asked and answered).
    pub async fn remember_conversation(
        &self,
        user_id: &str,
        session_id: Option<&str>,
        summary: &str,
        content: &str,
    ) -> Result<String, StoreError> {
        let id = new_id(id_prefix::EPISODE);
        let content: String = content.chars().take(EPISODE_MAX_CHARS).collect();
        self.store
            .add_episode(
                user_id,
                Episode {
                    id: id.clone(),
                    session_id: session_id.map(str::to_owned),
                    summary: summary.to_owned(),
                    content: content.clone(),
                    created_at: now_ms(),
                },
            )
            .await?;
        self.embed_and_store(
            user_id,
            MemoryKind::Episode,
            &id,
            format!("{summary}\n{content}"),
        )
        .await;
        Ok(id)
    }

    /// Facts and past conversations matching `query`, best first.
    pub async fn search(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Recall>, StoreError> {
        // Rankings from each source: (kind, id) -> position.
        let mut fused: HashMap<(MemoryKind, String), f32> = HashMap::new();
        let text_hits = self
            .store
            .search_memory_text(user_id, query, CANDIDATES_PER_SOURCE)
            .await?;
        for (rank, hit) in text_hits.into_iter().enumerate() {
            *fused.entry((hit.kind, hit.id)).or_default() += rrf(rank);
        }
        if let Some(mut q) = self.embed(user_id, vec![query.to_owned()]).await
            && let Some(q) = q.pop()
        {
            let stored = self.store.embeddings(user_id, &self.model).await?;
            let mut scored: Vec<(f32, MemoryKind, String)> = stored
                .into_iter()
                .map(|v| (cosine(&q, &v.vector), v.kind, v.id))
                .filter(|(s, _, _)| *s >= MIN_SIMILARITY)
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            scored.truncate(CANDIDATES_PER_SOURCE as usize);
            for (rank, (_, kind, id)) in scored.into_iter().enumerate() {
                *fused.entry((kind, id)).or_default() += rrf(rank);
            }
        }

        let mut ranked: Vec<((MemoryKind, String), f32)> = fused.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked.truncate(limit);

        let fact_ids: Vec<String> = ranked
            .iter()
            .filter(|((k, _), _)| *k == MemoryKind::Knowledge)
            .map(|((_, id), _)| id.clone())
            .collect();
        let episode_ids: Vec<String> = ranked
            .iter()
            .filter(|((k, _), _)| *k == MemoryKind::Episode)
            .map(|((_, id), _)| id.clone())
            .collect();
        let facts = self.store.get_knowledge(user_id, &fact_ids).await?;
        let episodes = self.store.get_episodes(user_id, &episode_ids).await?;

        let mut out = Vec::new();
        for ((kind, id), score) in ranked {
            match kind {
                MemoryKind::Knowledge => {
                    if let Some(f) = facts.iter().find(|f| f.id == id) {
                        out.push(Recall {
                            kind,
                            id,
                            title: f.topic.clone(),
                            text: f.content.clone(),
                            at: f.updated_at,
                            score,
                        });
                    }
                }
                MemoryKind::Episode => {
                    if let Some(e) = episodes.iter().find(|e| e.id == id) {
                        out.push(Recall {
                            kind,
                            id,
                            title: e.summary.clone(),
                            text: e.content.clone(),
                            at: e.created_at,
                            score,
                        });
                    }
                }
            }
        }
        Ok(out)
    }
}

fn rrf(rank: usize) -> f32 {
    1.0 / (RRF_K + rank as f32 + 1.0)
}

/// A topic reduced to its meaningful words, sorted, so "Favourite drink (updated)" and
/// "favourite drink" match.
pub fn topic_key(topic: &str) -> String {
    let mut words: Vec<String> = topic
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !TOPIC_FILLER_WORDS.contains(w))
        .map(str::to_owned)
        .collect();
    words.sort();
    words.dedup();
    words.join(" ")
}

/// Cosine similarity of two vectors (0 when either is empty or their sizes differ).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_match_without_filler_words() {
        assert_eq!(
            topic_key("favourite drink updated"),
            topic_key("Favourite drink")
        );
        assert_eq!(topic_key("My editor (new)"), topic_key("editor"));
        assert_ne!(topic_key("editor font size"), topic_key("editor"));
        assert_eq!(topic_key("updated"), "");
    }
    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
    }

    #[test]
    fn earlier_ranks_count_more() {
        assert!(rrf(0) > rrf(1) && rrf(1) > rrf(10));
    }
}

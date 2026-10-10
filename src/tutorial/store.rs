//! Save zone — the **only** path tutorial artifacts take onto disk
//! (D-0007), and therefore the cache that keeps LLM spend at one call
//! per game.
//!
//! Layout under the configured tutorial root (resolved by the caller,
//! typically `util::resolve_dir(Path::new("./tutorial"))` — sibling of
//! the history root):
//!
//! ```text
//! <root>/
//!   reviews/
//!     <game_ulid>.json      # one ReviewDoc per reviewed game
//! ```
//!
//! Writes funnel through a single mutex (same shape as
//! `crate::history::store`); reads re-open the file lock-free. A cache
//! file is authoritative until the user asks for a regeneration — the
//! pipeline checks [`TutorialStore::load_review`] before any provider
//! call.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{bail, Context, Result};

use crate::tutorial::material::ReviewDoc;

const REVIEWS_SUBDIR: &str = "reviews";

pub struct TutorialStore {
    root: PathBuf,
    /// Serialises writes; reads re-open the file each time.
    write_lock: Mutex<()>,
}

impl TutorialStore {
    /// Open (or create) the store at `root`, creating `reviews/` if
    /// missing.
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join(REVIEWS_SUBDIR))
            .with_context(|| format!("failed to create tutorial dir {}", root.display()))?;
        Ok(Self {
            root,
            write_lock: Mutex::new(()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn review_path(&self, game_id: &str) -> Result<PathBuf> {
        validate_game_id(game_id)?;
        Ok(self
            .root
            .join(REVIEWS_SUBDIR)
            .join(format!("{game_id}.json")))
    }

    /// Persist one review document, overwriting any earlier review of
    /// the same game (regeneration).
    pub fn save_review(&self, doc: &ReviewDoc) -> Result<()> {
        let _g = self
            .write_lock
            .lock()
            .expect("tutorial write lock poisoned");
        let path = self.review_path(&doc.game_id)?;
        let body = serde_json::to_string_pretty(doc).context("serialise ReviewDoc")?;
        fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    /// Load the cached review for a game, if one exists. A corrupt cache
    /// file is an error (the caller may regenerate); absence is `None`.
    pub fn load_review(&self, game_id: &str) -> Result<Option<ReviewDoc>> {
        let path = self.review_path(game_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let body = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let doc: ReviewDoc = serde_json::from_str(&body)
            .with_context(|| format!("corrupt review cache at {}", path.display()))?;
        Ok(Some(doc))
    }

    /// Drop the cached review for a game (no-op when absent).
    pub fn delete_review(&self, game_id: &str) -> Result<()> {
        let _g = self
            .write_lock
            .lock()
            .expect("tutorial write lock poisoned");
        let path = self.review_path(game_id)?;
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to delete {}", path.display()))?;
        }
        Ok(())
    }
}

/// Cache keys double as file names, so they must stay path-safe: ULIDs
/// from the history store always pass; anything with separators or
/// path-traversal dots never reaches the filesystem.
fn validate_game_id(game_id: &str) -> Result<()> {
    let ok = !game_id.is_empty()
        && game_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        bail!("illegal game id {game_id:?}: expected a ULID-style token");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tutorial::material::{EngineNumbers, TurnDoc};

    fn temp_root(tag: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "akagi-tutorial-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample_doc() -> ReviewDoc {
        ReviewDoc {
            game_id: "01JTESTGAME".into(),
            seat: 2,
            model: Some("test-model".into()),
            created_at: "2026-10-10T00:00:00Z".into(),
            turns: vec![TurnDoc {
                junme: 6,
                recommended: "dahai:5s".into(),
                numbers: EngineNumbers {
                    shanten: 0,
                    win_rate: Some(0.42),
                    deal_in_rate: Some(0.09),
                    avg_score: Some(5200),
                },
                explanation_zh: "切5s后听牌。".into(),
                headline: Some("进攻".into()),
            }],
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let root = temp_root("roundtrip");
        let store = TutorialStore::new(root.clone()).unwrap();

        assert_eq!(store.load_review("01JTESTGAME").unwrap(), None);

        let doc = sample_doc();
        store.save_review(&doc).unwrap();
        assert_eq!(store.load_review("01JTESTGAME").unwrap().unwrap(), doc);

        // Regeneration overwrites cleanly.
        let mut doc2 = doc.clone();
        doc2.model = Some("other-model".into());
        store.save_review(&doc2).unwrap();
        assert_eq!(store.load_review("01JTESTGAME").unwrap().unwrap(), doc2);

        store.delete_review("01JTESTGAME").unwrap();
        assert_eq!(store.load_review("01JTESTGAME").unwrap(), None);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rejects_path_unsafe_game_ids() {
        let root = temp_root("pathsafety");
        let store = TutorialStore::new(root.clone()).unwrap();
        for bad in ["", "../escape", "a/b", "a\\b", "ok..but.."] {
            assert!(
                store.load_review(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        std::fs::remove_dir_all(root).ok();
    }
}

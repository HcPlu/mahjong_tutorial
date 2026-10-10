//! Process-wide tutorial state shared by IPC commands (fork-only).
//!
//! Holds the save-zone store and the live coach's latest-decision cell.
//! Constructed once in `lib.rs` and handed to `AppState`; commands
//! borrow from here instead of re-opening the store per call.

use std::path::PathBuf;

use anyhow::Result;

use crate::tutorial::live::LiveCoachHandle;
use crate::tutorial::store::TutorialStore;

pub struct TutorialContext {
    pub store: TutorialStore,
    /// The most recent live decision point (bounded single slot).
    pub live: LiveCoachHandle,
}

impl TutorialContext {
    /// Open (or create) the tutorial store under `root`
    /// (`<config_root>/tutorial`, sibling of the history root).
    pub fn new(root: PathBuf, live: LiveCoachHandle) -> Result<Self> {
        Ok(Self {
            store: TutorialStore::new(root)?,
            live,
        })
    }
}

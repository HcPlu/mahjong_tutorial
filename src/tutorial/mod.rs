//! `src/tutorial/` — the AI coaching layer built on top of the existing
//! history / analysis machinery (fork-only module, see D-0003/D-0007).
//!
//! The module is organised as **three data zones with a one-way flow**
//! (invariants in the module README):
//!
//! ```text
//! ReviewMaterial (numbers from local engines)
//!   → upload   (redact + gate + render prompt)   — only exit point
//!   → provider (LLM call, OpenAI-compatible)     — only network point
//!   → download (parse + validate + cross-check)  — only entry point
//!   → store    (cache to reviews/<ulid>.json)    — only disk point
//! ```
//!
//! Nothing here touches `crate::ipc` or the internals of `crate::bot`;
//! the M1 review pipeline (IPC commands, frontend) is built *on top of*
//! these seams later.

pub mod download;
pub mod material;
pub mod provider;
pub mod store;
pub mod upload;

pub use download::parse_review_doc;
pub use material::{
    CandidateNumbers, EngineNumbers, ReviewDoc, ReviewMaterial, TurnDoc, TurnMaterial,
};
pub use provider::{
    ChatMessage, ChatRequest, ChatResponse, OpenAiCompatProvider, TutorialProvider,
};
pub use store::TutorialStore;
pub use upload::{build_messages, outbound_allowed, redact};

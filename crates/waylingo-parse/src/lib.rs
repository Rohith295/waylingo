//! The LLM step of Waylingo.
//!
//! Turns the end user's text into a validated [`waylingo_plan::Plan`] by asking an
//! OpenAI-compatible chat endpoint to fill a [`waylingo_plan::SurfaceIntent`], then
//! compiling it. Invalid output is sent back with the validation errors, a limited
//! number of times.
//!
//! # The rule this crate enforces
//!
//! Map provider data never reaches the LLM. This crate does not depend on any
//! provider code, and every message it can put in an LLM request is built from one
//! of four sources only:
//!
//! 1. the fixed system prompt,
//! 2. a [`ParseInput`] (query text, locale, saved-place names; no coordinates),
//! 3. the LLM's own earlier reply,
//! 4. [`waylingo_plan::PlanError`]s about that reply.
//!
//! The engine calls [`Parser`] before any map request exists, and stops using it
//! once the plan is done.

mod config;
mod conversation;
mod ground;
mod input;
mod openai;
mod parser;
mod prompt;

pub use config::{LlmConfig, LlmMode, PromptStyle};
pub use ground::ground_in_query;
pub use input::{InputError, ParseInput};
pub use parser::{FailureCode, ParseFailure, ParseStep, Parser};
pub use prompt::{COMPACT_SYSTEM_PROMPT, SYSTEM_PROMPT};

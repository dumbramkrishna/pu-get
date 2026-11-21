//! `pu-get` — browse and download official study material for Karnataka,
//! CBSE and NCERT boards from the terminal.
//!
//! The crate is split into a library and a thin binary so integration tests can
//! exercise the catalog, resolver and download layers directly.
//!
//! Modules:
//!
//! - [`app`] — TUI state machine and event loop
//! - [`ui`] — pure screen rendering
//! - [`cli`] — command-line parsing and dispatch
//! - [`config`] — persistent configuration and platform paths
//! - [`catalog`] — catalog model, loading and filtering
//! - [`download`] — streaming download manager
//! - [`http`] — shared HTTP client construction
//! - [`resolver`] — official-source resolvers and registry
//! - [`verify`] — catalog URL validation
//! - [`error`] — domain error types
#![forbid(unsafe_code)]

pub mod app;
pub mod catalog;
pub mod cli;
pub mod config;
pub mod download;
pub mod error;
pub mod http;
pub mod resolver;
pub mod ui;
pub mod verify;

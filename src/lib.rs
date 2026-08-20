//! hew — a config-driven viewer for mixed structured/plain log streams.
//!
//! Central contract: JSON object lines are formatted per `config.toml`,
//! everything else passes through verbatim — nothing in the stream may stop
//! the viewer.

pub mod config;
pub mod models;
pub mod service;

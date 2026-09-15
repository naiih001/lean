pub mod agent;
pub mod agents;
pub mod approval;
pub mod bash_guard;
pub mod dictate;
pub mod dir_guard;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod models;
pub mod question;
pub mod skills;
pub mod sudo;
pub mod tools;

// New foldered modules (1:1)
pub mod core;
pub mod guards;
pub mod integrations;
pub mod services;
pub mod support;
pub mod tui;

// Compat re-exports — keep old crate:: paths working during refactor
pub use core::context;
pub use integrations::herdr;
pub use integrations::observer;
pub use services::session;
pub use support::telemetry;
pub use tui::theme;

// New foldered modules (1:1) — 6 top-level domains
pub mod core;
pub mod guards;
pub mod integrations;
pub mod services;
pub mod support;
pub mod tools;
pub mod tui;

// Shims for backward compat — keep old crate:: paths alive during migration
pub mod agent;
pub mod llm;
pub mod mcp;

// Compat re-exports — keep old crate:: paths working during refactor
pub use core::context;
pub use guards::approval;
pub use guards::bash as bash_guard;
pub use guards::dir as dir_guard;
pub use guards::sudo;
pub use integrations::dictate;
pub use integrations::herdr;
pub use integrations::models;
pub use integrations::observer;
pub use services::agents;
pub use services::memory;
pub use services::question;
pub use services::session;
pub use services::skills;
pub use support::telemetry;
pub use tui::theme;

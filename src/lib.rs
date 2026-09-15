pub mod agent;
pub mod llm;
pub mod mcp;
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
pub use guards::approval;
pub use guards::bash as bash_guard;
pub use guards::dir as dir_guard;
pub use guards::sudo;
pub use integrations::dictate;
pub use integrations::herdr;
pub use integrations::models as models;
pub use integrations::observer;
pub use services::agents as agents;
pub use services::memory as memory;
pub use services::question as question;
pub use services::session;
pub use services::skills as skills;
pub use support::telemetry;
pub use tui::theme;

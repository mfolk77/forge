pub mod builtins;
pub mod catalog;
pub mod hooks;
pub mod manifest;
pub mod manager;
pub mod registry;
pub mod skill_loader;
pub mod tool_bridge;

pub use manager::PluginManager;
pub use registry::MarketplaceRegistry;

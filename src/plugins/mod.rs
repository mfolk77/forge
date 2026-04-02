pub mod builtins;
pub mod catalog;
pub mod hooks;
pub mod manifest;
pub mod manager;
pub mod registry;
pub mod skill_loader;
pub mod tool_bridge;

pub use manager::PluginManager;
pub use manifest::PluginManifest;
pub use registry::{MarketplacePlugin, MarketplaceRegistry, MarketplaceSource};
pub use skill_loader::LoadedSkill;
pub use tool_bridge::PluginTool;
pub use hooks::ResolvedHook;

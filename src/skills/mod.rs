pub mod builtin;
pub mod loader;

/// A loaded skill that can be triggered via slash command.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    /// Short name (e.g. "commit", "review")
    pub name: String,
    /// One-line description shown in help
    pub description: String,
    /// Slash-command trigger (e.g. "/commit")
    pub trigger: String,
    /// Markdown content injected into context when triggered
    pub content: String,
    /// Origin — "builtin" for shipped skills, plugin name otherwise
    pub source: String,
}

use super::LoadedSkill;
use super::builtin::builtin_skills;

/// Loads all skills: built-in first, then merges plugin skills.
/// Plugin skills override builtins that share the same trigger.
/// Unknown plugin skills are appended.
pub fn load_all_skills(plugin_skills: Vec<LoadedSkill>) -> Vec<LoadedSkill> {
    let mut all = builtin_skills();

    for ps in plugin_skills {
        if let Some(idx) = all.iter().position(|s| s.trigger == ps.trigger) {
            // Plugin overrides the builtin with the same trigger
            all[idx] = ps;
        } else {
            all.push(ps);
        }
    }

    all
}

/// Find a skill by its trigger string (e.g. "/commit").
pub fn find_skill_by_trigger<'a>(skills: &'a [LoadedSkill], trigger: &str) -> Option<&'a LoadedSkill> {
    skills.iter().find(|s| s.trigger == trigger)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_all_skills_returns_builtins() {
        let skills = load_all_skills(vec![]);
        assert!(!skills.is_empty());
        assert!(skills.iter().all(|s| s.source == "builtin"));
    }

    #[test]
    fn test_plugin_overrides_builtin() {
        let plugin_skill = LoadedSkill {
            name: "commit".to_string(),
            description: "Custom commit skill".to_string(),
            trigger: "/commit".to_string(),
            content: "Custom commit content from plugin".to_string(),
            source: "my-plugin".to_string(),
        };

        let skills = load_all_skills(vec![plugin_skill]);

        let commit = skills.iter().find(|s| s.trigger == "/commit").unwrap();
        assert_eq!(commit.source, "my-plugin");
        assert_eq!(commit.content, "Custom commit content from plugin");

        // Count of /commit triggers should be exactly 1 (replaced, not duplicated)
        let commit_count = skills.iter().filter(|s| s.trigger == "/commit").count();
        assert_eq!(commit_count, 1);
    }

    #[test]
    fn test_unknown_plugin_skill_appended() {
        let plugin_skill = LoadedSkill {
            name: "deploy".to_string(),
            description: "Deploy to production".to_string(),
            trigger: "/deploy".to_string(),
            content: "Deployment instructions".to_string(),
            source: "ops-plugin".to_string(),
        };

        let skills = load_all_skills(vec![plugin_skill]);

        let deploy = skills.iter().find(|s| s.trigger == "/deploy");
        assert!(deploy.is_some());
        assert_eq!(deploy.unwrap().source, "ops-plugin");

        // Builtins should still be present
        assert!(skills.iter().any(|s| s.trigger == "/commit"));
        assert!(skills.iter().any(|s| s.trigger == "/review"));
    }

    #[test]
    fn test_multiple_plugin_overrides_and_additions() {
        let plugins = vec![
            LoadedSkill {
                name: "commit".to_string(),
                description: "Override commit".to_string(),
                trigger: "/commit".to_string(),
                content: "Plugin commit".to_string(),
                source: "plugin-a".to_string(),
            },
            LoadedSkill {
                name: "lint".to_string(),
                description: "Linting skill".to_string(),
                trigger: "/lint".to_string(),
                content: "Lint instructions".to_string(),
                source: "plugin-b".to_string(),
            },
        ];

        let skills = load_all_skills(plugins);

        // /commit overridden
        let commit = skills.iter().find(|s| s.trigger == "/commit").unwrap();
        assert_eq!(commit.source, "plugin-a");

        // /lint added
        let lint = skills.iter().find(|s| s.trigger == "/lint").unwrap();
        assert_eq!(lint.source, "plugin-b");

        // Other builtins untouched
        let review = skills.iter().find(|s| s.trigger == "/review").unwrap();
        assert_eq!(review.source, "builtin");
    }

    #[test]
    fn test_find_skill_by_trigger() {
        let skills = load_all_skills(vec![]);

        assert!(find_skill_by_trigger(&skills, "/commit").is_some());
        assert!(find_skill_by_trigger(&skills, "/review").is_some());
        assert!(find_skill_by_trigger(&skills, "/nonexistent").is_none());
    }
}

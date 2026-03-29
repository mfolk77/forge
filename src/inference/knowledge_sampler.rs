use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TokenCandidate {
    pub id: i32,
    pub logit: f32,
}

#[derive(Debug, Clone, Default)]
pub struct EntityTrie {
    children: HashMap<i32, EntityTrie>,
    entity_id: Option<usize>,
}

impl EntityTrie {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, token_ids: &[i32], entity_id: usize) {
        let mut node = self;
        for &tid in token_ids {
            node = node.children.entry(tid).or_default();
        }
        node.entity_id = Some(entity_id);
    }

    pub fn step(&self, token_id: i32) -> Option<&EntityTrie> {
        self.children.get(&token_id)
    }

    pub fn matched_entity(&self) -> Option<usize> {
        self.entity_id
    }

    pub fn is_prefix(&self) -> bool {
        !self.children.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct Fact {
    pub text: String,
    pub token_ids: Vec<i32>,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct MitosisNode {
    pub name: String,
    pub facts: Vec<Fact>,
    pub children: HashMap<String, MitosisNode>,
}

impl MitosisNode {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            facts: Vec::new(),
            children: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MitosisTree {
    root: MitosisNode,
}

impl MitosisTree {
    pub fn new() -> Self {
        Self {
            root: MitosisNode::new("root"),
        }
    }

    pub fn insert(&mut self, path: &[&str], fact: Fact) {
        let mut node = &mut self.root;
        for &segment in path {
            node = node
                .children
                .entry(segment.to_string())
                .or_insert_with(|| MitosisNode::new(segment));
        }
        node.facts.push(fact);
    }

    pub fn navigate(&self, path: &[&str]) -> Option<&[Fact]> {
        let mut node = &self.root;
        for &segment in path {
            node = node.children.get(segment)?;
        }
        if node.facts.is_empty() {
            None
        } else {
            Some(&node.facts)
        }
    }
}

impl Default for MitosisTree {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SamplerState {
    Idle,
    Matching,
    EntityMatched,
    InjectingFact,
}

const SUPPRESSION_PATTERNS: &[&str] = &[
    "unlike",
    "compared to",
    "rather than",
    "instead of",
    "not to be confused with",
    "as opposed to",
];

#[derive(Debug, Clone)]
pub struct KnowledgeSampler {
    trie: EntityTrie,
    tree: MitosisTree,
    entities: Vec<EntityEntry>,
    state: SamplerState,
    trie_cursor: Option<usize>,
    matched_entity: Option<usize>,
    fact_inject_pos: usize,
    current_fact_tokens: Vec<i32>,
    token_buffer: Vec<i32>,
    text_buffer: String,
    logit_boost: f32,
}

#[derive(Debug, Clone)]
struct EntityEntry {
    name: String,
    path: Vec<String>,
}

impl KnowledgeSampler {
    pub fn new(trie: EntityTrie, tree: MitosisTree) -> Self {
        Self {
            trie,
            tree,
            entities: Vec::new(),
            state: SamplerState::Idle,
            trie_cursor: None,
            matched_entity: None,
            fact_inject_pos: 0,
            current_fact_tokens: Vec::new(),
            token_buffer: Vec::new(),
            text_buffer: String::new(),
            logit_boost: 12.0,
        }
    }

    pub fn with_logit_boost(mut self, boost: f32) -> Self {
        self.logit_boost = boost;
        self
    }

    pub fn register_entity(&mut self, name: &str, token_ids: &[i32], path: Vec<String>) -> usize {
        let id = self.entities.len();
        self.trie.insert(token_ids, id);
        self.entities.push(EntityEntry {
            name: name.to_string(),
            path,
        });
        id
    }

    pub fn accept(&mut self, token_id: i32) {
        self.token_buffer.push(token_id);

        match self.state {
            SamplerState::Idle => {
                if let Some(next) = self.trie.step(token_id) {
                    self.trie_cursor = Some(token_id as usize);
                    if let Some(eid) = next.matched_entity() {
                        self.transition_to_matched(eid);
                    } else if next.is_prefix() {
                        self.state = SamplerState::Matching;
                    }
                }
            }
            SamplerState::Matching => {
                // Walk the trie from root following the accumulated token_buffer tail
                if let Some(node) = self.walk_trie_from_recent() {
                    if let Some(eid) = node.matched_entity() {
                        self.transition_to_matched(eid);
                    } else if !node.is_prefix() {
                        self.reset_to_idle();
                    }
                } else {
                    self.reset_to_idle();
                }
            }
            SamplerState::EntityMatched => {
                if self.is_suppressed() {
                    self.reset_to_idle();
                } else if is_fact_context(&self.text_buffer) {
                    self.begin_injection();
                }
            }
            SamplerState::InjectingFact => {
                self.fact_inject_pos += 1;
                if self.fact_inject_pos >= self.current_fact_tokens.len() {
                    self.reset_to_idle();
                }
            }
        }
    }

    pub fn apply(&self, candidates: &mut Vec<TokenCandidate>) {
        if self.state != SamplerState::InjectingFact {
            return;
        }
        if self.fact_inject_pos >= self.current_fact_tokens.len() {
            return;
        }

        let target = self.current_fact_tokens[self.fact_inject_pos];
        for c in candidates.iter_mut() {
            if c.id == target {
                c.logit += self.logit_boost;
            }
        }
    }

    pub fn state_name(&self) -> &'static str {
        match self.state {
            SamplerState::Idle => "idle",
            SamplerState::Matching => "matching",
            SamplerState::EntityMatched => "entity_matched",
            SamplerState::InjectingFact => "injecting_fact",
        }
    }

    fn transition_to_matched(&mut self, entity_id: usize) {
        self.state = SamplerState::EntityMatched;
        self.matched_entity = Some(entity_id);
    }

    fn begin_injection(&mut self) {
        if let Some(eid) = self.matched_entity {
            if let Some(entry) = self.entities.get(eid) {
                let path_refs: Vec<&str> = entry.path.iter().map(|s| s.as_str()).collect();
                if let Some(facts) = self.tree.navigate(&path_refs) {
                    if let Some(best) = facts.iter().max_by(|a, b| {
                        a.confidence
                            .partial_cmp(&b.confidence)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    }) {
                        self.current_fact_tokens = best.token_ids.clone();
                        self.fact_inject_pos = 0;
                        self.state = SamplerState::InjectingFact;
                        return;
                    }
                }
            }
        }
        self.reset_to_idle();
    }

    fn reset_to_idle(&mut self) {
        self.state = SamplerState::Idle;
        self.trie_cursor = None;
        self.matched_entity = None;
        self.fact_inject_pos = 0;
        self.current_fact_tokens.clear();
    }

    fn is_suppressed(&self) -> bool {
        let lower = self.text_buffer.to_lowercase();
        SUPPRESSION_PATTERNS.iter().any(|p| lower.ends_with(p))
    }

    fn walk_trie_from_recent(&self) -> Option<&EntityTrie> {
        // Walk from the end of token_buffer backwards to find longest trie path
        let buf = &self.token_buffer;
        let max_depth = buf.len().min(32);
        for start in (buf.len().saturating_sub(max_depth))..buf.len() {
            let mut node = &self.trie;
            let mut valid = true;
            for &tid in &buf[start..] {
                match node.step(tid) {
                    Some(next) => node = next,
                    None => {
                        valid = false;
                        break;
                    }
                }
            }
            if valid {
                return Some(node);
            }
        }
        None
    }
}

pub fn is_fact_context(buffer: &str) -> bool {
    if buffer.len() < 3 {
        return false;
    }
    let lower = buffer.to_lowercase();
    let tail = if lower.len() > 80 { &lower[lower.len() - 80..] } else { &lower };

    let fact_signals = [
        " is ", " is", " are ", " are", " was ", " was", " were ", " were",
        " has ", " has", " had ", " had",
        " means ", " means", " refers to ", " refers to",
        " defined as ", " defined as", " known as ", " known as",
        " equals ", " equals", " contains ", " contains",
        " consists of ", " consists of", " located in ", " located in",
        " founded in ", " founded in", " created by ", " created by",
        " developed by ", " developed by",
    ];

    fact_signals.iter().any(|s| tail.ends_with(s) || tail.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trie() -> EntityTrie {
        let mut trie = EntityTrie::new();
        trie.insert(&[100, 200, 300], 0);
        trie.insert(&[100, 200, 400], 1);
        trie.insert(&[500], 2);
        trie
    }

    #[test]
    fn test_trie_insert_and_lookup() {
        let trie = make_trie();
        let n1 = trie.step(100).unwrap();
        assert!(n1.matched_entity().is_none());
        assert!(n1.is_prefix());

        let n2 = n1.step(200).unwrap();
        assert!(n2.is_prefix());

        let n3 = n2.step(300).unwrap();
        assert_eq!(n3.matched_entity(), Some(0));

        let n4 = n2.step(400).unwrap();
        assert_eq!(n4.matched_entity(), Some(1));
    }

    #[test]
    fn test_trie_single_token_entity() {
        let trie = make_trie();
        let n = trie.step(500).unwrap();
        assert_eq!(n.matched_entity(), Some(2));
    }

    #[test]
    fn test_trie_miss() {
        let trie = make_trie();
        assert!(trie.step(999).is_none());
    }

    #[test]
    fn test_mitosis_tree_insert_and_navigate() {
        let mut tree = MitosisTree::new();
        tree.insert(
            &["science", "physics"],
            Fact {
                text: "E=mc^2".to_string(),
                token_ids: vec![10, 20, 30],
                confidence: 0.95,
            },
        );
        tree.insert(
            &["science", "physics"],
            Fact {
                text: "F=ma".to_string(),
                token_ids: vec![40, 50],
                confidence: 0.9,
            },
        );

        let facts = tree.navigate(&["science", "physics"]).unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].text, "E=mc^2");
    }

    #[test]
    fn test_mitosis_tree_navigate_missing() {
        let tree = MitosisTree::new();
        assert!(tree.navigate(&["nonexistent"]).is_none());
    }

    #[test]
    fn test_knowledge_sampler_idle_state() {
        let ks = KnowledgeSampler::new(EntityTrie::new(), MitosisTree::new());
        assert_eq!(ks.state_name(), "idle");
    }

    #[test]
    fn test_knowledge_sampler_single_token_match() {
        let mut trie = EntityTrie::new();
        trie.insert(&[42], 0);

        let mut tree = MitosisTree::new();
        tree.insert(
            &["test"],
            Fact {
                text: "fact".to_string(),
                token_ids: vec![1, 2, 3],
                confidence: 1.0,
            },
        );

        let mut ks = KnowledgeSampler::new(trie, tree);
        ks.register_entity("test_entity", &[42], vec!["test".to_string()]);

        // Feed the matching token
        ks.accept(42);
        assert_eq!(ks.state_name(), "entity_matched");
    }

    #[test]
    fn test_knowledge_sampler_apply_boosts_logit() {
        let mut trie = EntityTrie::new();
        trie.insert(&[42], 0);

        let mut tree = MitosisTree::new();
        tree.insert(
            &["test"],
            Fact {
                text: "fact".to_string(),
                token_ids: vec![10, 20],
                confidence: 1.0,
            },
        );

        let mut ks = KnowledgeSampler::new(trie, tree);
        ks.register_entity("e", &[42], vec!["test".to_string()]);
        ks.accept(42);

        // Force into injection state
        ks.text_buffer = "the entity is ".to_string();
        ks.accept(42); // triggers fact context check -> injection

        let mut candidates = vec![
            TokenCandidate { id: 10, logit: 1.0 },
            TokenCandidate { id: 99, logit: 1.0 },
        ];
        ks.apply(&mut candidates);

        if ks.state_name() == "injecting_fact" {
            assert!(candidates[0].logit > candidates[1].logit);
        }
    }

    #[test]
    fn test_knowledge_sampler_no_boost_when_idle() {
        let ks = KnowledgeSampler::new(EntityTrie::new(), MitosisTree::new());
        let mut candidates = vec![
            TokenCandidate { id: 1, logit: 5.0 },
            TokenCandidate { id: 2, logit: 3.0 },
        ];
        let original: Vec<f32> = candidates.iter().map(|c| c.logit).collect();
        ks.apply(&mut candidates);
        for (c, orig) in candidates.iter().zip(original.iter()) {
            assert!((c.logit - orig).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn test_is_fact_context_positive() {
        assert!(is_fact_context("the capital of France is "));
        assert!(is_fact_context("this compound was developed by "));
        assert!(is_fact_context("the elements are "));
        assert!(is_fact_context("this concept refers to "));
    }

    #[test]
    fn test_is_fact_context_negative() {
        assert!(!is_fact_context(""));
        assert!(!is_fact_context("ab"));
        assert!(!is_fact_context("hello world"));
        assert!(!is_fact_context("just some random text here"));
    }

    #[test]
    fn test_suppression_patterns() {
        let mut trie = EntityTrie::new();
        trie.insert(&[42], 0);

        let mut tree = MitosisTree::new();
        tree.insert(
            &["test"],
            Fact {
                text: "fact".to_string(),
                token_ids: vec![1],
                confidence: 1.0,
            },
        );

        let mut ks = KnowledgeSampler::new(trie, tree);
        ks.register_entity("e", &[42], vec!["test".to_string()]);

        ks.accept(42);
        assert_eq!(ks.state_name(), "entity_matched");

        ks.text_buffer = "this is unlike".to_string();
        ks.accept(0); // non-matching token in EntityMatched state with suppression
        assert_eq!(ks.state_name(), "idle");
    }

    #[test]
    fn test_configurable_logit_boost() {
        let trie = EntityTrie::new();
        let tree = MitosisTree::new();
        let ks = KnowledgeSampler::new(trie, tree).with_logit_boost(8.0);
        assert!((ks.logit_boost - 8.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_register_entity() {
        let mut ks = KnowledgeSampler::new(EntityTrie::new(), MitosisTree::new());
        let id0 = ks.register_entity("Rust", &[10, 20], vec!["lang".to_string()]);
        let id1 = ks.register_entity("Python", &[30, 40], vec!["lang".to_string()]);
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
    }
}

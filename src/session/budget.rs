/// Token budget management for context window allocation.
///
/// Splits the model's context window into regions:
/// - System reserved: system prompt, rules, memory
/// - Conversation: user/assistant messages
/// - Tool results: capped output from tool invocations
/// - Generation reserve: headroom for the model's reply
#[derive(Debug, Clone)]
pub struct TokenBudget {
    /// Total context window size in tokens.
    pub total: usize,
    /// Tokens reserved for system prompt and rules.
    pub system_reserved: usize,
    /// Tokens reserved for generation output (default 4096).
    pub generation_reserve: usize,
}

impl TokenBudget {
    pub fn new(total: usize, system_reserved: usize) -> Self {
        Self {
            total,
            system_reserved,
            generation_reserve: 4096,
        }
    }

    pub fn with_generation_reserve(mut self, reserve: usize) -> Self {
        self.generation_reserve = reserve;
        self
    }

    /// Tokens available for conversation history (messages + tool results).
    pub fn available_for_conversation(&self) -> usize {
        self.total
            .saturating_sub(self.system_reserved)
            .saturating_sub(self.generation_reserve)
    }

    /// Tokens available for a single tool result, given current conversation usage.
    /// Capped at 2000 tokens to prevent any one tool result from dominating context.
    pub fn available_for_tool_results(&self, conversation_tokens: usize) -> usize {
        let remaining = self
            .available_for_conversation()
            .saturating_sub(conversation_tokens);
        remaining.min(2000)
    }

    /// Whether the conversation should be compacted (summarized).
    /// Returns true when conversation tokens exceed 80% of available space.
    pub fn should_compact(&self, conversation_tokens: usize) -> bool {
        let available = self.available_for_conversation();
        if available == 0 {
            return true;
        }
        conversation_tokens > available * 80 / 100
    }

    /// Estimate token count for a string using the chars/4 heuristic.
    /// This is intentionally simple — accurate counting requires the actual tokenizer.
    pub fn estimate_tokens(text: &str) -> usize {
        // Roughly 4 bytes per token for English text.
        // Use len() (bytes) not chars().count() — byte length is closer to
        // actual tokenizer behavior for mixed content.
        (text.len() + 3) / 4
    }

    /// Truncate a tool result using middle-out truncation.
    ///
    /// Keeps the beginning and end of the result (which usually contain the most
    /// useful context) and replaces the middle with a truncation marker.
    pub fn truncate_tool_result(result: &str, max_tokens: usize) -> String {
        let max_chars = max_tokens * 4; // Inverse of estimate_tokens heuristic.
        if result.len() <= max_chars {
            return result.to_string();
        }

        if max_chars < 40 {
            // Too small for meaningful middle-out; just take the head.
            return result.chars().take(max_chars).collect();
        }

        let marker = "\n\n... [truncated] ...\n\n";
        let marker_len = marker.len();
        let usable = max_chars.saturating_sub(marker_len);
        let head_len = usable * 60 / 100; // 60% head, 40% tail
        let tail_len = usable - head_len;

        let head: String = result.chars().take(head_len).collect();
        let tail: String = result
            .chars()
            .rev()
            .take(tail_len)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        format!("{head}{marker}{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_for_conversation() {
        let budget = TokenBudget::new(32000, 4000);
        // 32000 - 4000 - 4096 = 23904
        assert_eq!(budget.available_for_conversation(), 23904);
    }

    #[test]
    fn test_available_for_conversation_tiny_context() {
        let budget = TokenBudget::new(2000, 1500);
        // 2000 - 1500 - 4096 = saturates to 0
        assert_eq!(budget.available_for_conversation(), 0);
    }

    #[test]
    fn test_available_for_tool_results_capped_at_2000() {
        let budget = TokenBudget::new(128000, 4000);
        // Plenty of room, but capped at 2000.
        assert_eq!(budget.available_for_tool_results(0), 2000);
    }

    #[test]
    fn test_available_for_tool_results_limited_by_remaining() {
        let budget = TokenBudget::new(32000, 4000);
        let avail = budget.available_for_conversation(); // 23904
                                                         // Almost full: only 500 tokens left.
        let tool_budget = budget.available_for_tool_results(avail - 500);
        assert_eq!(tool_budget, 500);
    }

    #[test]
    fn test_should_compact_below_threshold() {
        let budget = TokenBudget::new(32000, 4000);
        let avail = budget.available_for_conversation();
        // At 70% — should not compact.
        assert!(!budget.should_compact(avail * 70 / 100));
    }

    #[test]
    fn test_should_compact_above_threshold() {
        let budget = TokenBudget::new(32000, 4000);
        let avail = budget.available_for_conversation();
        // At 85% — should compact.
        assert!(budget.should_compact(avail * 85 / 100));
    }

    #[test]
    fn test_should_compact_at_boundary() {
        let budget = TokenBudget::new(32000, 4000);
        let avail = budget.available_for_conversation();
        // Exactly 80% — should NOT compact (needs to exceed 80%).
        assert!(!budget.should_compact(avail * 80 / 100));
        // 80% + 1 — should compact.
        assert!(budget.should_compact(avail * 80 / 100 + 1));
    }

    #[test]
    fn test_should_compact_zero_available() {
        let budget = TokenBudget::new(100, 100);
        assert!(budget.should_compact(0));
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(TokenBudget::estimate_tokens(""), 0); // (0+3)/4 = 0
        assert_eq!(TokenBudget::estimate_tokens("hi"), 1); // (2+3)/4 = 1
        assert_eq!(TokenBudget::estimate_tokens("hello world"), 3); // (11+3)/4 = 3
                                                                    // Longer text.
        let long = "a".repeat(400);
        assert_eq!(TokenBudget::estimate_tokens(&long), 100); // (400+3)/4 = 100
    }

    #[test]
    fn test_truncate_tool_result_short() {
        let short = "hello world";
        let result = TokenBudget::truncate_tool_result(short, 100);
        assert_eq!(result, short);
    }

    #[test]
    fn test_truncate_tool_result_long() {
        let long = "a".repeat(10000);
        let result = TokenBudget::truncate_tool_result(&long, 100);
        // max_chars = 400. Should be truncated.
        assert!(result.len() <= 400 + 30); // Allow slight overhead from marker.
        assert!(result.contains("[truncated]"));
        // Should start with 'a's and end with 'a's.
        assert!(result.starts_with("aaa"));
        assert!(result.ends_with("aaa"));
    }

    #[test]
    fn test_truncate_preserves_head_and_tail() {
        let text = format!("HEAD{}TAIL", "x".repeat(10000));
        let result = TokenBudget::truncate_tool_result(&text, 50);
        assert!(result.starts_with("HEAD"));
        assert!(result.ends_with("TAIL"));
    }

    #[test]
    fn test_truncate_very_small_budget() {
        let text = "a".repeat(1000);
        let result = TokenBudget::truncate_tool_result(&text, 5);
        // max_chars = 20, too small for middle-out, takes head.
        assert_eq!(result.len(), 20);
    }

    #[test]
    fn test_generation_reserve_custom() {
        let budget = TokenBudget::new(32000, 4000).with_generation_reserve(8192);
        // 32000 - 4000 - 8192 = 19808
        assert_eq!(budget.available_for_conversation(), 19808);
    }

    #[test]
    fn test_estimate_tokens_empty() {
        // Edge case: (0 + 3) / 4 = 0 via integer division.
        assert_eq!(TokenBudget::estimate_tokens(""), 0);
    }
}

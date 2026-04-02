use anyhow::{bail, Result};
use tokio::sync::mpsc;

use crate::backend::types::{ChatResponse, Message, Role, StopReason, Token, TokenUsage};
use super::context::LlamaContext;
use super::knowledge_sampler::{KnowledgeSampler, TokenCandidate};

#[derive(Debug, Clone)]
pub struct Sampler {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub repeat_penalty: f32,
    pub repeat_window: usize,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            repeat_penalty: 1.1,
            repeat_window: 64,
        }
    }
}

impl Sampler {
    pub async fn generate_stream(
        &self,
        ctx: &LlamaContext,
        prompt_tokens: &[i32],
        max_tokens: usize,
        mut knowledge: Option<&mut KnowledgeSampler>,
        tx: mpsc::Sender<Token>,
    ) -> Result<ChatResponse> {
        if prompt_tokens.is_empty() {
            bail!("empty prompt");
        }

        ctx.decode_batch(prompt_tokens, 0)?;

        let mut generated = Vec::with_capacity(max_tokens);
        let mut output_text = String::new();
        let mut pos = prompt_tokens.len() as i32;

        for _ in 0..max_tokens {
            let logits = ctx.get_logits();
            if logits.is_empty() {
                break;
            }

            let mut candidates: Vec<TokenCandidate> = logits
                .iter()
                .enumerate()
                .map(|(id, &logit)| TokenCandidate {
                    id: id as i32,
                    logit,
                })
                .collect();

            self.apply_repeat_penalty(&mut candidates, &generated);
            self.apply_temperature(&mut candidates);

            if let Some(ks) = knowledge.as_deref_mut() {
                ks.apply(&mut candidates);
            }

            let token_id = self.sample_top_p_k(&mut candidates);

            // EOS check (token 2 is common EOS for most GGUF models)
            if token_id == 2 {
                let _ = tx
                    .send(Token {
                        text: String::new(),
                        is_final: true,
                    })
                    .await;
                return Ok(build_response(
                    &output_text,
                    prompt_tokens.len(),
                    generated.len(),
                    StopReason::EndOfText,
                ));
            }

            let piece = ctx.token_to_str(token_id);
            output_text.push_str(&piece);
            generated.push(token_id);

            if let Some(ks) = knowledge.as_deref_mut() {
                ks.accept(token_id);
            }

            let send_result = tx
                .send(Token {
                    text: piece,
                    is_final: false,
                })
                .await;

            if send_result.is_err() {
                break; // receiver dropped
            }

            ctx.decode_batch(&[token_id], pos)?;
            pos += 1;
        }

        let _ = tx
            .send(Token {
                text: String::new(),
                is_final: true,
            })
            .await;

        Ok(build_response(
            &output_text,
            prompt_tokens.len(),
            generated.len(),
            StopReason::MaxTokens,
        ))
    }

    fn apply_repeat_penalty(&self, candidates: &mut [TokenCandidate], recent: &[i32]) {
        let window_start = recent.len().saturating_sub(self.repeat_window);
        let window = &recent[window_start..];

        for candidate in candidates.iter_mut() {
            if window.contains(&candidate.id) {
                if candidate.logit > 0.0 {
                    candidate.logit /= self.repeat_penalty;
                } else {
                    candidate.logit *= self.repeat_penalty;
                }
            }
        }
    }

    fn apply_temperature(&self, candidates: &mut [TokenCandidate]) {
        if self.temperature > 0.0 {
            for c in candidates.iter_mut() {
                c.logit /= self.temperature;
            }
        }
    }

    fn sample_top_p_k(&self, candidates: &mut Vec<TokenCandidate>) -> i32 {
        candidates.sort_unstable_by(|a, b| b.logit.partial_cmp(&a.logit).unwrap_or(std::cmp::Ordering::Equal));

        let k = self.top_k as usize;
        if candidates.len() > k {
            candidates.truncate(k);
        }

        // Softmax
        let max_logit = candidates.first().map(|c| c.logit).unwrap_or(0.0);
        let mut probs: Vec<f32> = candidates.iter().map(|c| (c.logit - max_logit).exp()).collect();
        let sum: f32 = probs.iter().sum();
        if sum > 0.0 {
            for p in &mut probs {
                *p /= sum;
            }
        }

        // Top-p nucleus sampling
        let mut cumulative = 0.0;
        let mut cutoff = probs.len();
        for (i, &p) in probs.iter().enumerate() {
            cumulative += p;
            if cumulative >= self.top_p {
                cutoff = i + 1;
                break;
            }
        }
        let probs = &probs[..cutoff];
        let candidates = &candidates[..cutoff];

        // Weighted random selection using simple deterministic fallback
        // (in production this would use a proper RNG)
        let r: f32 = simple_random_float();
        let mut acc = 0.0;
        for (i, &p) in probs.iter().enumerate() {
            acc += p;
            if acc >= r {
                return candidates[i].id;
            }
        }

        candidates.last().map(|c| c.id).unwrap_or(0)
    }
}

fn simple_random_float() -> f32 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 10000) as f32 / 10000.0
}

fn build_response(
    text: &str,
    prompt_tokens: usize,
    completion_tokens: usize,
    stop_reason: StopReason,
) -> ChatResponse {
    ChatResponse {
        message: Message {
            role: Role::Assistant,
            content: text.to_string(),
            tool_calls: None,
            tool_call_id: None,
        },
        tokens_used: TokenUsage {
            prompt_tokens,
            completion_tokens,
        },
        stop_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_sampler() {
        let s = Sampler::default();
        assert!((s.temperature - 0.7).abs() < f32::EPSILON);
        assert!((s.top_p - 0.9).abs() < f32::EPSILON);
        assert_eq!(s.top_k, 40);
        assert!((s.repeat_penalty - 1.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_apply_temperature() {
        let s = Sampler {
            temperature: 2.0,
            ..Default::default()
        };
        let mut candidates = vec![
            TokenCandidate { id: 0, logit: 4.0 },
            TokenCandidate { id: 1, logit: 2.0 },
        ];
        s.apply_temperature(&mut candidates);
        assert!((candidates[0].logit - 2.0).abs() < f32::EPSILON);
        assert!((candidates[1].logit - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_apply_repeat_penalty() {
        let s = Sampler {
            repeat_penalty: 2.0,
            repeat_window: 10,
            ..Default::default()
        };
        let recent = vec![5, 10];
        let mut candidates = vec![
            TokenCandidate { id: 5, logit: 4.0 },
            TokenCandidate { id: 10, logit: -2.0 },
            TokenCandidate { id: 7, logit: 3.0 },
        ];
        s.apply_repeat_penalty(&mut candidates, &recent);
        assert!((candidates[0].logit - 2.0).abs() < f32::EPSILON); // positive / penalty
        assert!((candidates[1].logit - -4.0).abs() < f32::EPSILON); // negative * penalty
        assert!((candidates[2].logit - 3.0).abs() < f32::EPSILON); // untouched
    }

    #[test]
    fn test_sample_top_p_k_returns_valid_id() {
        let s = Sampler::default();
        let mut candidates = vec![
            TokenCandidate { id: 0, logit: 10.0 },
            TokenCandidate { id: 1, logit: 5.0 },
            TokenCandidate { id: 2, logit: 1.0 },
        ];
        let id = s.sample_top_p_k(&mut candidates);
        assert!((0..=2).contains(&id));
    }

    #[test]
    fn test_sample_greedy_at_zero_temp() {
        let s = Sampler {
            temperature: 0.01, // near-zero for near-greedy
            top_k: 1,
            top_p: 1.0,
            ..Default::default()
        };
        let mut candidates = vec![
            TokenCandidate { id: 42, logit: 100.0 },
            TokenCandidate { id: 1, logit: 0.1 },
            TokenCandidate { id: 2, logit: 0.01 },
        ];
        let id = s.sample_top_p_k(&mut candidates);
        assert_eq!(id, 42);
    }

    #[test]
    fn test_build_response() {
        let resp = build_response("hello", 10, 5, StopReason::EndOfText);
        assert_eq!(resp.message.content, "hello");
        assert_eq!(resp.message.role, Role::Assistant);
        assert_eq!(resp.tokens_used.prompt_tokens, 10);
        assert_eq!(resp.tokens_used.completion_tokens, 5);
        assert_eq!(resp.stop_reason, StopReason::EndOfText);
    }
}

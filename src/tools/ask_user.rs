use anyhow::Result;
use serde_json::Value;

use super::registry::{Tool, ToolContext, ToolResult};

pub struct AskUserTool;

impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "ask_user"
    }

    fn description(&self) -> &str {
        "Ask the user a question and wait for their response."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["question"],
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional list of choices"
                }
            }
        })
    }

    fn execute(
        &self,
        params: Value,
        _ctx: &ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        let question = params["question"].as_str().unwrap_or("").to_string();
        let options = params["options"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            });

        Box::pin(async move {
            if question.is_empty() {
                return Ok(ToolResult::error("No question provided"));
            }

            // In the TUI, this will be handled by the UI layer.
            // For now, use stdin/stdout directly.
            println!("\n{question}");

            if let Some(opts) = &options {
                for (i, opt) in opts.iter().enumerate() {
                    println!("  {}. {opt}", i + 1);
                }
                println!();
            }

            print!("> ");
            use std::io::Write;
            std::io::stdout().flush().unwrap();

            let mut input = String::new();
            std::io::stdin().read_line(&mut input).unwrap();
            let input = input.trim().to_string();

            // If options were given and user entered a number, resolve it
            if let Some(opts) = options {
                if let Ok(idx) = input.parse::<usize>() {
                    if idx >= 1 && idx <= opts.len() {
                        return Ok(ToolResult::success(&opts[idx - 1]));
                    }
                }
            }

            Ok(ToolResult::success(input))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ask_user_params() {
        let tool = AskUserTool;
        let params = tool.parameters();
        assert!(params["required"].as_array().unwrap().contains(&Value::String("question".to_string())));
    }
}

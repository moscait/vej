//! Input limits, pre-load shape validation, and batch token-budget planning.

use std::ops::Range;

use super::choices::build_choices;
use super::types::{
    BatchDecisionRequest, ChoiceItem, DecisionRequest, InvalidRequest, QuestionSpec,
};

pub const MAX_LETTERED_CHOICES: usize = 26;
pub const MAX_SCORE_LEVELS: usize = 101;
pub const MAX_QUESTIONS_PER_REQUEST: usize = 32;
pub const MAX_BATCH_ITEMS: usize = 256;
pub const MAX_BATCH_TASKS: usize = 1024;
pub const MAX_BATCH_SEQS: usize = 32;
pub const MAX_BATCH_TOKENS: usize = 8192;
pub const MIN_GENERATE_CTX: u32 = 8192;
pub const MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;

pub const LETTER_LABELS: [&str; MAX_LETTERED_CHOICES] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z",
];

impl DecisionRequest {
    /// Validates request shape and size limits without loading a model.
    pub fn validate(&self) -> Result<(), InvalidRequest> {
        if let Some(ref questions) = self.questions {
            validate_questions_map(questions)?;
            Ok(())
        } else if let Some(ref choices) = self.choices {
            validate_flat_choices(choices)
        } else {
            Err(InvalidRequest(
                "Request must provide either 'questions' (Jev format) or 'choices' (flat format)"
                    .to_string(),
            ))
        }
    }
}

impl BatchDecisionRequest {
    /// Validates batch request shape and size limits without loading a model.
    pub fn validate(&self) -> Result<(), InvalidRequest> {
        if self.items.len() > MAX_BATCH_ITEMS {
            return Err(InvalidRequest(format!(
                "Batch may contain at most {MAX_BATCH_ITEMS} items (got {})",
                self.items.len()
            )));
        }

        if let Some(ref questions) = self.questions {
            validate_questions_map(questions)?;
        }
        if let Some(ref choices) = self.choices {
            validate_flat_choices(choices)?;
        }

        let mut total_tasks: usize = 0;
        for (idx, item) in self.items.iter().enumerate() {
            if let Some(ref questions) = item.questions {
                let count = validate_questions_map(questions)?;
                total_tasks = total_tasks.saturating_add(count);
            } else if let Some(ref choices) = item.choices {
                validate_flat_choices(choices)?;
                total_tasks = total_tasks.saturating_add(1);
            } else if let Some(ref questions) = self.questions {
                total_tasks = total_tasks.saturating_add(questions.len());
            } else if self.choices.is_some() {
                total_tasks = total_tasks.saturating_add(1);
            } else {
                return Err(InvalidRequest(format!(
                    "Item {idx} must provide either 'choices' or 'questions'"
                )));
            }

            if total_tasks > MAX_BATCH_TASKS {
                return Err(InvalidRequest(format!(
                    "Batch may evaluate at most {MAX_BATCH_TASKS} total item-questions (got {total_tasks})"
                )));
            }
        }

        Ok(())
    }
}

pub(crate) fn validate_flat_choices(choices: &[String]) -> Result<(), InvalidRequest> {
    if choices.is_empty() {
        return Err(InvalidRequest("Choices list cannot be empty".to_string()));
    }
    if choices.len() > MAX_LETTERED_CHOICES {
        return Err(InvalidRequest(format!(
            "At most {MAX_LETTERED_CHOICES} choices (A-Z) are supported (got {})",
            choices.len()
        )));
    }
    Ok(())
}

pub(crate) fn validate_choice_count(
    choices: &[ChoiceItem],
    is_score: bool,
) -> Result<(), InvalidRequest> {
    if choices.is_empty() {
        return Err(InvalidRequest("Choices list cannot be empty".to_string()));
    }
    let is_boolean = choices.len() == 2
        && choices.iter().any(|c| c.key.eq_ignore_ascii_case("true"))
        && choices.iter().any(|c| c.key.eq_ignore_ascii_case("false"));
    let is_numeric =
        is_score && !is_boolean && choices.iter().all(|c| c.key.parse::<u32>().is_ok());

    if is_numeric {
        if choices.len() > MAX_SCORE_LEVELS {
            return Err(InvalidRequest(format!(
                "At most {MAX_SCORE_LEVELS} score levels (0..=100) are supported (got {})",
                choices.len()
            )));
        }
    } else if choices.len() > MAX_LETTERED_CHOICES {
        return Err(InvalidRequest(format!(
            "At most {MAX_LETTERED_CHOICES} lettered choices (A-Z) are supported (got {})",
            choices.len()
        )));
    }
    Ok(())
}

pub(crate) fn validate_questions_map(
    questions: &serde_json::Map<String, serde_json::Value>,
) -> Result<usize, InvalidRequest> {
    if questions.is_empty() {
        return Err(InvalidRequest(
            "'questions' map cannot be empty".to_string(),
        ));
    }
    if questions.len() > MAX_QUESTIONS_PER_REQUEST {
        return Err(InvalidRequest(format!(
            "At most {MAX_QUESTIONS_PER_REQUEST} questions are allowed per request (got {})",
            questions.len()
        )));
    }
    for (q_name, q_val) in questions {
        let q_spec: QuestionSpec = serde_json::from_value(q_val.clone())
            .map_err(|e| InvalidRequest(format!("Failed to parse question '{q_name}': {e}")))?;
        build_choices(&q_spec).map_err(|e| InvalidRequest(format!("Question '{q_name}' {e}")))?;
    }
    Ok(questions.len())
}

pub(crate) fn check_prompt_tokens(n_tokens: usize, limit: usize) -> Result<(), InvalidRequest> {
    if n_tokens > limit {
        return Err(InvalidRequest(format!(
            "input is {n_tokens} tokens; the limit is {limit}"
        )));
    }
    Ok(())
}

pub(crate) fn generation_context_cap(n_ctx: u32, n_ctx_train: u32) -> u32 {
    n_ctx.max(MIN_GENERATE_CTX).min(n_ctx_train).max(1)
}

/// Groups prompt token counts into contiguous chunks bounded by both sequence count and token budget.
pub(crate) fn plan_batch_chunks(
    token_counts: &[usize],
    max_seqs: usize,
    max_tokens: usize,
) -> Vec<Range<usize>> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut seqs = 0;
    let mut tokens = 0usize;

    for (idx, &len) in token_counts.iter().enumerate() {
        let exceeds_seqs = seqs >= max_seqs;
        let exceeds_tokens = seqs > 0 && tokens.saturating_add(len) > max_tokens;
        if exceeds_seqs || exceeds_tokens {
            chunks.push(start..idx);
            start = idx;
            seqs = 0;
            tokens = 0;
        }
        seqs += 1;
        tokens = tokens.saturating_add(len);
    }

    if start < token_counts.len() {
        chunks.push(start..token_counts.len());
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn check_prompt_tokens_formats_exact_error_message() {
        assert!(check_prompt_tokens(2048, 2048).is_ok());
        let err = check_prompt_tokens(2500, 2048).unwrap_err();
        assert_eq!(err.0, "input is 2500 tokens; the limit is 2048");
    }

    #[test]
    fn generation_context_cap_respects_min_8192_and_n_ctx_train() {
        assert_eq!(generation_context_cap(2048, 131_072), 8192);
        assert_eq!(generation_context_cap(16_384, 131_072), 16_384);
        assert_eq!(generation_context_cap(2048, 4096), 4096);
    }

    #[test]
    fn plan_batch_chunks_bounds_sequences_and_token_budget() {
        let chunks = plan_batch_chunks(&[3000, 3000, 3000, 1000], 32, 8192);
        assert_eq!(chunks, vec![0..2, 2..4]);

        let many_short = vec![10; 70];
        let chunks = plan_batch_chunks(&many_short, 32, 8192);
        assert_eq!(chunks, vec![0..32, 32..64, 64..70]);
    }

    #[test]
    fn validate_accepts_32_questions_and_1024_total_tasks() {
        let mut q32 = serde_json::Map::new();
        for i in 0..32 {
            q32.insert(format!("q{i}"), json!({ "type": "noul" }));
        }
        let req = DecisionRequest {
            model: None,
            state: json!("s"),
            choices: None,
            instruction: None,
            questions: Some(q32.clone()),
            temperature: 1.0,
            keep_alive: None,
        };
        assert!(req.validate().is_ok());

        let items: Vec<_> = (0..32)
            .map(|i| super::super::types::BatchItem {
                id: format!("item-{i}"),
                state: json!("s"),
                choices: None,
                instruction: None,
                questions: None,
            })
            .collect();
        let batch = BatchDecisionRequest {
            model: None,
            choices: None,
            instruction: None,
            questions: Some(q32),
            items,
            temperature: 1.0,
            keep_alive: None,
        };
        assert!(batch.validate().is_ok());
    }
}

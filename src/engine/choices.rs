//! Pure question-to-choice parsing helpers (independent of llama.cpp types).

use serde_json::Value;

use super::types::{ChoiceItem, InvalidRequest, QuestionSpec};

/// Temporary split mode while preserving exact legacy behavior before Step 2 unification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChoiceMode {
    Single,
    Batch,
}

const LEGACY_LETTERS: [&str; 16] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P",
];

/// Parses a `QuestionSpec`'s criteria into concrete candidate choices.
pub fn build_choices(spec: &QuestionSpec) -> Result<Vec<ChoiceItem>, InvalidRequest> {
    build_choices_with_mode(spec, ChoiceMode::Single)
}

pub(crate) fn build_choices_with_mode(
    spec: &QuestionSpec,
    mode: ChoiceMode,
) -> Result<Vec<ChoiceItem>, InvalidRequest> {
    let mut choice_items = Vec::new();
    let is_score = spec.q_type.eq_ignore_ascii_case("score");

    let effective_crit = spec
        .criteria
        .as_ref()
        .or(spec.options.as_ref())
        .or(spec.choices.as_ref())
        .or(spec.levels.as_ref());

    if let Some(crit) = effective_crit {
        append_criteria_choices(crit, is_score, mode, &mut choice_items);
    }

    if choice_items.is_empty() {
        apply_default_choices(spec, is_score, mode, &mut choice_items)?;
    }

    Ok(choice_items)
}

fn append_criteria_choices(
    crit: &Value,
    is_score: bool,
    mode: ChoiceMode,
    choice_items: &mut Vec<ChoiceItem>,
) {
    match crit {
        Value::Object(map) => {
            for (k, v) in map {
                let desc = v.as_str().unwrap_or("");
                let mut display = k.clone();
                if !desc.is_empty() {
                    display.push_str(": ");
                    display.push_str(desc);
                }
                choice_items.push(ChoiceItem {
                    key: k.clone(),
                    display,
                });
            }
        }
        Value::Array(arr) => {
            for (idx, item) in arr.iter().enumerate() {
                let text = item.as_str().unwrap_or("");
                let (key, display) = match mode {
                    ChoiceMode::Single => {
                        let key = if is_score {
                            idx.to_string()
                        } else {
                            text.to_string()
                        };
                        let display = if is_score {
                            if text.is_empty() {
                                idx.to_string()
                            } else {
                                format!("{}: {}", idx, text)
                            }
                        } else {
                            text.to_string()
                        };
                        (key, display)
                    }
                    ChoiceMode::Batch => {
                        let key = if is_score {
                            idx.to_string()
                        } else {
                            LEGACY_LETTERS.get(idx).copied().unwrap_or("?").to_string()
                        };
                        (key, text.to_string())
                    }
                };
                choice_items.push(ChoiceItem { key, display });
            }
        }
        Value::Number(num) if is_score && mode == ChoiceMode::Single => {
            let n = num.as_u64().unwrap_or(6) as usize;
            for lvl in 0..n {
                choice_items.push(ChoiceItem {
                    key: lvl.to_string(),
                    display: format!("Level {}", lvl),
                });
            }
        }
        _ => {}
    }
}

fn apply_default_choices(
    spec: &QuestionSpec,
    is_score: bool,
    mode: ChoiceMode,
    choice_items: &mut Vec<ChoiceItem>,
) -> Result<(), InvalidRequest> {
    if is_score && mode == ChoiceMode::Single {
        for lvl in 0..=5 {
            choice_items.push(ChoiceItem {
                key: lvl.to_string(),
                display: format!("Level {}", lvl),
            });
        }
        return Ok(());
    }

    let is_boolean_like = spec.q_type.eq_ignore_ascii_case("noul")
        || (mode == ChoiceMode::Batch && spec.q_type.eq_ignore_ascii_case("boolean"));

    if is_boolean_like {
        choice_items.push(ChoiceItem {
            key: "true".to_string(),
            display: "true (yes)".to_string(),
        });
        choice_items.push(ChoiceItem {
            key: "false".to_string(),
            display: "false (no)".to_string(),
        });
        return Ok(());
    }

    Err(InvalidRequest(
        "must define criteria with options".to_string(),
    ))
}

/// Formats question instructions from a string, structured object, or arbitrary JSON value.
pub fn format_instructions(spec: &QuestionSpec) -> Option<String> {
    match &spec.instructions {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Object(map)) => {
            let mut parts = Vec::new();
            if let Some(q) = map.get("question").and_then(|v| v.as_str()) {
                parts.push(q.to_string());
            }
            for (k, v) in map {
                if k != "question" {
                    if let Some(s) = v.as_str() {
                        parts.push(format!("{k}: {s}"));
                    } else {
                        parts.push(format!("{k}: {v}"));
                    }
                }
            }
            Some(parts.join("\n"))
        }
        Some(val) => Some(val.to_string()),
        None => None,
    }
}

/// Converts a state JSON value into the prompt text representation.
pub(crate) fn format_state_str(state: &Value) -> Result<String, serde_json::Error> {
    match state {
        Value::String(s) => Ok(s.clone()),
        Value::Null => Ok(String::new()),
        val => serde_json::to_string_pretty(val),
    }
}

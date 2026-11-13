//! Pure question-to-choice parsing helpers (independent of llama.cpp types).

use serde_json::Value;

use super::limits::{MAX_SCORE_LEVELS, validate_choice_count};
use super::types::{ChoiceItem, InvalidRequest, QuestionSpec};

/// Parses a `QuestionSpec`'s criteria into concrete candidate choices.
pub fn build_choices(spec: &QuestionSpec) -> Result<Vec<ChoiceItem>, InvalidRequest> {
    let mut choice_items = Vec::new();
    let is_score = spec.q_type.eq_ignore_ascii_case("score");

    let effective_crit = spec
        .criteria
        .as_ref()
        .or(spec.options.as_ref())
        .or(spec.choices.as_ref())
        .or(spec.levels.as_ref());

    if let Some(crit) = effective_crit {
        append_criteria_choices(crit, is_score, &mut choice_items)?;
    }

    if choice_items.is_empty() {
        apply_default_choices(spec, is_score, &mut choice_items)?;
    }

    validate_choice_count(&choice_items, is_score)?;
    Ok(choice_items)
}

fn append_criteria_choices(
    crit: &Value,
    is_score: bool,
    choice_items: &mut Vec<ChoiceItem>,
) -> Result<(), InvalidRequest> {
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
                let (key, display) = if is_score {
                    let key = idx.to_string();
                    let display = if text.is_empty() {
                        idx.to_string()
                    } else {
                        text.to_string()
                    };
                    (key, display)
                } else {
                    (text.to_string(), text.to_string())
                };
                choice_items.push(ChoiceItem { key, display });
            }
        }
        Value::Number(num) if is_score => {
            let Some(n) = num.as_u64() else {
                return Err(InvalidRequest(format!(
                    "score levels must be an integer between 1 and {MAX_SCORE_LEVELS}"
                )));
            };
            if n == 0 || n > MAX_SCORE_LEVELS as u64 {
                return Err(InvalidRequest(format!(
                    "score levels must be between 1 and {MAX_SCORE_LEVELS} (got {n})"
                )));
            }
            for lvl in 0..(n as usize) {
                choice_items.push(ChoiceItem {
                    key: lvl.to_string(),
                    display: format!("Level {}", lvl),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

fn apply_default_choices(
    spec: &QuestionSpec,
    is_score: bool,
    choice_items: &mut Vec<ChoiceItem>,
) -> Result<(), InvalidRequest> {
    if is_score {
        for lvl in 0..=5 {
            choice_items.push(ChoiceItem {
                key: lvl.to_string(),
                display: format!("Level {}", lvl),
            });
        }
        return Ok(());
    }

    if spec.q_type.eq_ignore_ascii_case("noul") || spec.q_type.eq_ignore_ascii_case("boolean") {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_choices_canonical_table() {
        struct Case {
            name: &'static str,
            spec: Value,
            expected: Result<Vec<(&'static str, &'static str)>, &'static str>,
        }

        let cases = [
            Case {
                name: "choice with array criteria uses text for key and display",
                spec: json!({ "type": "choice", "criteria": ["Paris", "London"] }),
                expected: Ok(vec![("Paris", "Paris"), ("London", "London")]),
            },
            Case {
                name: "score without criteria defaults to levels 0..=5",
                spec: json!({ "type": "score" }),
                expected: Ok(vec![
                    ("0", "Level 0"),
                    ("1", "Level 1"),
                    ("2", "Level 2"),
                    ("3", "Level 3"),
                    ("4", "Level 4"),
                    ("5", "Level 5"),
                ]),
            },
            Case {
                name: "boolean without criteria defaults to true/false",
                spec: json!({ "type": "boolean" }),
                expected: Ok(vec![("true", "true (yes)"), ("false", "false (no)")]),
            },
            Case {
                name: "noul without criteria defaults to true/false",
                spec: json!({ "type": "noul" }),
                expected: Ok(vec![("true", "true (yes)"), ("false", "false (no)")]),
            },
            Case {
                name: "score with numeric levels expands 0..N",
                spec: json!({ "type": "score", "levels": 3 }),
                expected: Ok(vec![("0", "Level 0"), ("1", "Level 1"), ("2", "Level 2")]),
            },
            Case {
                name: "score with array criteria uses option text or index when empty",
                spec: json!({ "type": "score", "criteria": ["Poor", "", "Great"] }),
                expected: Ok(vec![("0", "Poor"), ("1", "1"), ("2", "Great")]),
            },
            Case {
                name: "map criteria preserves keys and formats non-empty string descriptions",
                spec: json!({
                    "type": "choice",
                    "criteria": { "refund": "Billing issue", "support": "" }
                }),
                expected: Ok(vec![
                    ("refund", "refund: Billing issue"),
                    ("support", "support"),
                ]),
            },
            Case {
                name: "choice with empty array criteria returns InvalidRequest",
                spec: json!({ "type": "choice", "criteria": [] }),
                expected: Err("must define criteria"),
            },
            Case {
                name: "choice with empty map criteria returns InvalidRequest",
                spec: json!({ "type": "choice", "criteria": {} }),
                expected: Err("must define criteria"),
            },
            Case {
                name: "score with empty array criteria falls back to defaults 0..=5",
                spec: json!({ "type": "score", "criteria": [] }),
                expected: Ok(vec![
                    ("0", "Level 0"),
                    ("1", "Level 1"),
                    ("2", "Level 2"),
                    ("3", "Level 3"),
                    ("4", "Level 4"),
                    ("5", "Level 5"),
                ]),
            },
            Case {
                name: "map criteria with non-string values leaves display as key",
                spec: json!({ "type": "choice", "criteria": { "a": 42, "b": null, "c": true } }),
                expected: Ok(vec![("a", "a"), ("b", "b"), ("c", "c")]),
            },
            Case {
                name: "score array with non-string values falls back to index display",
                spec: json!({ "type": "score", "criteria": [123, null, "Top"] }),
                expected: Ok(vec![("0", "0"), ("1", "1"), ("2", "Top")]),
            },
            Case {
                name: "non-score question with numeric criteria is rejected",
                spec: json!({ "type": "choice", "levels": 3 }),
                expected: Err("must define criteria"),
            },
            Case {
                name: "score with 101 levels (0..=100) succeeds",
                spec: json!({ "type": "score", "levels": 101 }),
                expected: Ok(vec![]), // checked separately below
            },
            Case {
                name: "score with absurd or oversized numeric levels is rejected",
                spec: json!({ "type": "score", "levels": 1_000_000_000_000_000_000_u64 }),
                expected: Err("between 1 and 101"),
            },
            Case {
                name: "score with 102 numeric levels is rejected",
                spec: json!({ "type": "score", "levels": 102 }),
                expected: Err("between 1 and 101"),
            },
            Case {
                name: "score with zero levels is rejected",
                spec: json!({ "type": "score", "levels": 0 }),
                expected: Err("between 1 and 101"),
            },
        ];

        for case in cases {
            let spec: QuestionSpec = serde_json::from_value(case.spec).expect("valid QuestionSpec");
            let actual = build_choices(&spec);
            if case.name == "score with 101 levels (0..=100) succeeds" {
                let items = actual.expect("101 levels valid");
                assert_eq!(items.len(), 101);
                assert_eq!(items.first().unwrap().key, "0");
                assert_eq!(items.last().unwrap().key, "100");
                continue;
            }
            match case.expected {
                Ok(expected_pairs) => {
                    let items =
                        actual.unwrap_or_else(|e| panic!("{}: unexpected err: {e}", case.name));
                    let got_pairs: Vec<(&str, &str)> = items
                        .iter()
                        .map(|c| (c.key.as_str(), c.display.as_str()))
                        .collect();
                    assert_eq!(got_pairs, expected_pairs, "{}", case.name);
                }
                Err(substr) => {
                    let err = actual
                        .err()
                        .unwrap_or_else(|| panic!("{}: expected err", case.name));
                    assert!(
                        err.0.contains(substr),
                        "{}: expected error containing '{substr}', got '{}'",
                        case.name,
                        err
                    );
                }
            }
        }
    }
}

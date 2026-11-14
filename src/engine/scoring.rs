//! Logit scoring and probability normalization for candidate choices.

use llama_cpp_2::model::LlamaModel;
use std::collections::{HashMap, HashSet};

use super::limits::LETTER_LABELS;
use super::types::{AnswerOutput, ChoiceItem, InvalidRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LabelScheme {
    Boolean,
    Numeric,
    Letters,
}

#[derive(Debug, Clone)]
pub(crate) struct ScoringPlan {
    pub scheme: LabelScheme,
    pub token_ids: Vec<Vec<i32>>,
}

pub(crate) trait PieceTokenizer {
    fn tokenize_piece(&self, piece: &str) -> Option<Vec<i32>>;
}

impl PieceTokenizer for LlamaModel {
    fn tokenize_piece(&self, piece: &str) -> Option<Vec<i32>> {
        let tokens = self.vocab().tokenize(piece.as_bytes(), false, true);
        Some(tokens.into_iter().map(|t| t.0).collect())
    }
}

fn variants_for_boolean(key: &str) -> [&'static str; 4] {
    if key.eq_ignore_ascii_case("true") {
        [" true", "true", " True", "True"]
    } else {
        [" false", "false", " False", "False"]
    }
}

fn variants_for_numeric(key: &str) -> [String; 2] {
    let k = key.trim();
    [format!(" {k}"), k.to_string()]
}

fn variants_for_letter(letter: &str) -> [String; 4] {
    [
        format!(" {letter}"),
        letter.to_string(),
        format!(" {}", letter.to_lowercase()),
        letter.to_lowercase(),
    ]
}

fn collect_single_tokens<'a>(
    tokenizer: &impl PieceTokenizer,
    variants: impl IntoIterator<Item = &'a str>,
) -> Vec<i32> {
    let mut single_tokens = Vec::new();
    for variant in variants {
        if let Some(tokens) = tokenizer.tokenize_piece(variant)
            && tokens.len() == 1
            && !single_tokens.contains(&tokens[0])
        {
            single_tokens.push(tokens[0]);
        }
    }
    single_tokens
}

fn all_disjoint_and_non_empty(per_choice: &[Vec<i32>]) -> bool {
    let mut seen = HashSet::new();
    for tokens in per_choice {
        if tokens.is_empty() {
            return false;
        }
        for &tok in tokens {
            if !seen.insert(tok) {
                return false;
            }
        }
    }
    true
}

pub(crate) fn resolve_scoring_plan(
    tokenizer: &impl PieceTokenizer,
    choices: &[ChoiceItem],
) -> Result<ScoringPlan, InvalidRequest> {
    let is_boolean_keys = choices.len() == 2
        && choices.iter().any(|c| c.key.eq_ignore_ascii_case("true"))
        && choices.iter().any(|c| c.key.eq_ignore_ascii_case("false"));
    if is_boolean_keys {
        let per_choice: Vec<Vec<i32>> = choices
            .iter()
            .map(|c| collect_single_tokens(tokenizer, variants_for_boolean(&c.key)))
            .collect();
        if all_disjoint_and_non_empty(&per_choice) {
            return Ok(ScoringPlan {
                scheme: LabelScheme::Boolean,
                token_ids: per_choice,
            });
        }
    }

    let is_numeric_keys =
        !is_boolean_keys && choices.iter().all(|c| c.key.trim().parse::<u32>().is_ok());
    if is_numeric_keys {
        let per_choice: Vec<Vec<i32>> = choices
            .iter()
            .map(|c| {
                let vars = variants_for_numeric(&c.key);
                collect_single_tokens(tokenizer, vars.iter().map(String::as_str))
            })
            .collect();
        if all_disjoint_and_non_empty(&per_choice) {
            return Ok(ScoringPlan {
                scheme: LabelScheme::Numeric,
                token_ids: per_choice,
            });
        }
    }

    if choices.len() > LETTER_LABELS.len() {
        return Err(InvalidRequest(format!(
            "Question has {} choices, but numeric keys cannot be represented as distinct single tokens on this model and lettered fallback supports at most {} choices (A-Z)",
            choices.len(),
            LETTER_LABELS.len()
        )));
    }

    let per_choice = (0..choices.len())
        .map(|idx| {
            let vars = variants_for_letter(LETTER_LABELS[idx]);
            collect_single_tokens(tokenizer, vars.iter().map(String::as_str))
        })
        .collect();
    Ok(ScoringPlan {
        scheme: LabelScheme::Letters,
        token_ids: per_choice,
    })
}

pub(crate) fn score_choices_from_logits(
    logits: &[f32],
    choices: &[ChoiceItem],
    plan: &ScoringPlan,
    temperature: f32,
) -> (String, f32, HashMap<String, f32>) {
    let mut candidate_logits: Vec<f32> = Vec::with_capacity(choices.len());

    for token_ids in &plan.token_ids {
        let mut best_logit = f32::NEG_INFINITY;
        for &tok_id in token_ids {
            if let Ok(tok_idx) = usize::try_from(tok_id)
                && let Some(&l) = logits.get(tok_idx)
                && l > best_logit
            {
                best_logit = l;
            }
        }

        if best_logit == f32::NEG_INFINITY {
            best_logit = -100.0;
        }

        candidate_logits.push(best_logit);
    }

    let temp = if temperature <= 0.0 { 1.0 } else { temperature };

    let max_logit = candidate_logits
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);

    let exp_sum: f32 = candidate_logits
        .iter()
        .map(|&l| ((l - max_logit) / temp).exp())
        .sum();

    let mut probabilities = HashMap::new();
    let mut best_decision = choices.first().map(|c| c.key.clone()).unwrap_or_default();
    let mut max_prob = -1.0_f32;

    for (idx, item) in choices.iter().enumerate() {
        let prob = if exp_sum > 0.0 {
            ((candidate_logits[idx] - max_logit) / temp).exp() / exp_sum
        } else {
            1.0 / (choices.len() as f32)
        };

        if prob > max_prob {
            max_prob = prob;
            best_decision = item.key.clone();
        }

        probabilities.insert(item.key.clone(), prob);
    }

    (best_decision, max_prob, probabilities)
}

pub(crate) fn build_answer_output(
    q_type: &str,
    is_score: bool,
    decision: String,
    confidence: f32,
    probabilities: HashMap<String, f32>,
) -> AnswerOutput {
    let noul_val = if q_type.eq_ignore_ascii_case("noul") {
        probabilities.get("true").copied()
    } else {
        None
    };

    let (score_val, display_choice) = if is_score {
        let mut sum_score = 0.0f32;
        for (k, prob) in &probabilities {
            if let Ok(lvl) = k.parse::<f32>() {
                sum_score += lvl * prob;
            }
        }
        let rounded = (sum_score * 100.0).round() / 100.0;
        (Some(rounded), Some(format!("{:.2}", rounded)))
    } else {
        (None, Some(decision))
    };

    AnswerOutput {
        q_type: q_type.to_string(),
        choice: display_choice,
        noul: noul_val,
        score: score_val,
        probabilities,
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeTokenizer {
        vocab: HashMap<String, Vec<i32>>,
    }

    impl FakeTokenizer {
        fn gemma_like() -> Self {
            let mut vocab = HashMap::new();
            // Space token = 1, digits 0..=9 = 10..=19, letters A..=Z / a..=z = 20..=75
            vocab.insert(" ".to_string(), vec![1]);
            for d in 0..=9 {
                vocab.insert(d.to_string(), vec![10 + d]);
                vocab.insert(format!(" {d}"), vec![1, 10 + d]);
            }
            // "10" splits into digits '1' (11) and '0' (10)
            vocab.insert("10".to_string(), vec![11, 10]);
            vocab.insert(" 10".to_string(), vec![1, 11, 10]);
            for idx in 0..=25 {
                let upper = char::from(b'A' + idx as u8).to_string();
                let lower = char::from(b'a' + idx as u8).to_string();
                vocab.insert(upper.clone(), vec![20 + idx]);
                vocab.insert(format!(" {upper}"), vec![1, 20 + idx]);
                vocab.insert(lower.clone(), vec![50 + idx]);
                vocab.insert(format!(" {lower}"), vec![1, 50 + idx]);
            }
            vocab.insert(" true".to_string(), vec![80]);
            vocab.insert("true".to_string(), vec![81]);
            vocab.insert(" false".to_string(), vec![82]);
            vocab.insert("false".to_string(), vec![83]);
            Self { vocab }
        }

        fn llama3_like() -> Self {
            let mut vocab = Self::gemma_like().vocab;
            // Llama 3 keeps multi-digit numbers up to 3 digits as single tokens
            vocab.insert("10".to_string(), vec![90]);
            // Even with space prefix multi-token, single-token "10" is unique
            vocab.insert(" 10".to_string(), vec![1, 90]);
            Self { vocab }
        }
    }

    impl PieceTokenizer for FakeTokenizer {
        fn tokenize_piece(&self, piece: &str) -> Option<Vec<i32>> {
            self.vocab.get(piece).cloned()
        }
    }

    fn numeric_choices(n: usize) -> Vec<ChoiceItem> {
        (0..n)
            .map(|i| ChoiceItem {
                key: i.to_string(),
                display: i.to_string(),
            })
            .collect()
    }

    #[test]
    fn gemma_digit_splitting_falls_back_to_letters_and_never_collides() {
        let tok = FakeTokenizer::gemma_like();
        let choices = numeric_choices(11); // 0..=10
        let plan = resolve_scoring_plan(&tok, &choices).expect("11 choices fit A-K");
        assert_eq!(plan.scheme, LabelScheme::Letters);

        // Make digit '1' logit (11) huge and letter 'K' (choice 10 -> id 30) highest among letters
        let mut logits = vec![-10.0; 100];
        logits[11] = 99.0;
        logits[30] = 5.0; // 'K'
        logits[21] = 1.0; // 'B'
        let (decision, _, probs) = score_choices_from_logits(&logits, &choices, &plan, 1.0);
        assert_eq!(decision, "10");
        assert!(probs["10"] > probs["1"]);
    }

    #[test]
    fn llama3_single_token_digits_use_numeric_and_ignore_multi_token_prefixes() {
        let tok = FakeTokenizer::llama3_like();
        let choices = numeric_choices(11); // 0..=10
        let plan = resolve_scoring_plan(&tok, &choices).expect("numeric resolves");
        assert_eq!(plan.scheme, LabelScheme::Numeric);

        // Space prefix token (id 1) is huge, "1" is 11, "10" is 90; multi-token " 1" / " 10" are ignored
        let mut logits = vec![-10.0; 100];
        logits[1] = 99.0;
        logits[11] = 1.0; // "1"
        logits[90] = 6.0; // "10"
        let (decision, _, probs) = score_choices_from_logits(&logits, &choices, &plan, 1.0);
        assert_eq!(decision, "10");
        assert!(probs["10"] > probs["1"]);
    }

    #[test]
    fn numeric_collision_across_keys_falls_back_to_letters() {
        let mut tok = FakeTokenizer::llama3_like();
        tok.vocab.insert("00".to_string(), vec![10]); // collides with "0"
        let choices = vec![
            ChoiceItem {
                key: "0".into(),
                display: "Zero".into(),
            },
            ChoiceItem {
                key: "00".into(),
                display: "Double zero".into(),
            },
        ];
        let plan = resolve_scoring_plan(&tok, &choices).expect("letter fallback");
        assert_eq!(plan.scheme, LabelScheme::Letters);
    }

    #[test]
    fn boolean_uses_boolean_when_both_have_single_tokens_else_letters() {
        let tok = FakeTokenizer::gemma_like();
        let bool_choices = vec![
            ChoiceItem {
                key: "true".into(),
                display: "True".into(),
            },
            ChoiceItem {
                key: "false".into(),
                display: "False".into(),
            },
        ];
        let plan = resolve_scoring_plan(&tok, &bool_choices).expect("boolean plan");
        assert_eq!(plan.scheme, LabelScheme::Boolean);

        // Remove single-token variants for "false" -> must fall back to Letters
        let mut split_tok = FakeTokenizer::gemma_like();
        split_tok.vocab.insert(" false".to_string(), vec![1, 82]);
        split_tok.vocab.insert("false".to_string(), vec![82, 83]);
        let fallback_plan =
            resolve_scoring_plan(&split_tok, &bool_choices).expect("letters fallback");
        assert_eq!(fallback_plan.scheme, LabelScheme::Letters);
    }

    #[test]
    fn gemma_letter_fallback_rejects_more_than_26_levels_with_invalid_request() {
        let tok = FakeTokenizer::gemma_like();
        let choices = numeric_choices(27);
        let err = resolve_scoring_plan(&tok, &choices).expect_err("exceeds 26 letters");
        assert!(err.0.contains("26"), "{err}");
    }
}

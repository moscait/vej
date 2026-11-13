//! Logit scoring and probability normalization for candidate choices.

use llama_cpp_2::model::{AddBos, LlamaModel};
use std::collections::HashMap;

use super::types::{AnswerOutput, ChoiceItem};

const LETTERS: [&str; 16] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P",
];

pub(crate) fn score_choices_from_logits(
    model: &LlamaModel,
    logits: &[f32],
    choices: &[ChoiceItem],
    temperature: f32,
) -> (String, f32, HashMap<String, f32>) {
    let is_boolean = choices.len() == 2
        && choices.iter().any(|c| c.key.eq_ignore_ascii_case("true"))
        && choices.iter().any(|c| c.key.eq_ignore_ascii_case("false"));

    let is_numeric = !is_boolean && choices.iter().all(|c| c.key.parse::<u32>().is_ok());
    let mut candidate_logits: Vec<f32> = Vec::with_capacity(choices.len());

    for (idx, item) in choices.iter().enumerate() {
        let mut variants = Vec::new();

        if is_boolean {
            if item.key.eq_ignore_ascii_case("true") {
                variants.push(" true".to_string());
                variants.push("true".to_string());
                variants.push(" True".to_string());
                variants.push("True".to_string());
            } else {
                variants.push(" false".to_string());
                variants.push("false".to_string());
                variants.push(" False".to_string());
                variants.push("False".to_string());
            }
        } else if is_numeric {
            let k = item.key.trim();
            variants.push(format!(" {k}"));
            variants.push(k.to_string());
        } else if let Some(&letter) = LETTERS.get(idx) {
            variants.push(format!(" {letter}"));
            variants.push(letter.to_string());
            variants.push(format!(" {}", letter.to_lowercase()));
            variants.push(letter.to_lowercase());
        }

        let mut best_logit = f32::NEG_INFINITY;
        for v in &variants {
            if let Ok(tokens) = model.str_to_token(v, AddBos::Never)
                && let Some(tok) = tokens.first()
            {
                let l = logits[tok.0 as usize];
                if l > best_logit {
                    best_logit = l;
                }
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

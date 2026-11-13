//! Prompt templates for decision scoring and text generation.

use llama_cpp_2::model::AddBos;

use crate::models::{TemplateStyle, find_model_spec};

use super::limits::LETTER_LABELS;
use super::scoring::LabelScheme;
use super::types::ChoiceItem;

pub(crate) fn resolve_template(
    model_id: &str,
    override_template: Option<TemplateStyle>,
) -> TemplateStyle {
    override_template.unwrap_or_else(|| {
        if let Some(spec) = find_model_spec(model_id) {
            spec.template
        } else if model_id.to_lowercase().contains("gemma-4")
            || model_id.to_lowercase().contains("gemma4")
        {
            TemplateStyle::Gemma4
        } else if model_id.to_lowercase().contains("llama") {
            TemplateStyle::Llama3
        } else if model_id.to_lowercase().contains("gemma") {
            TemplateStyle::Gemma
        } else {
            TemplateStyle::ChatML
        }
    })
}

pub(crate) fn add_bos_for(template: TemplateStyle) -> AddBos {
    match template {
        TemplateStyle::Llama3 | TemplateStyle::ChatML | TemplateStyle::Gemma4 => AddBos::Never,
        TemplateStyle::Gemma => AddBos::Always,
    }
}

pub(crate) fn format_decision_prompt(
    template: TemplateStyle,
    instruction: Option<&str>,
    state_str: &str,
    choices: &[ChoiceItem],
    scheme: LabelScheme,
) -> String {
    let (system_msg, formatted_options, selection_prompt) = match scheme {
        LabelScheme::Boolean => {
            let mut opts = String::new();
            for item in choices {
                opts.push_str(&format!("\n- {}: {}", item.key, item.display));
            }
            (
                "You are a precise, low-latency decision system. Evaluate the input state and criteria, then respond with true or false.",
                opts,
                "Select true or false:".to_string(),
            )
        }
        LabelScheme::Numeric => {
            let mut opts = String::new();
            for item in choices {
                opts.push_str(&format!("\n{}) {}", item.key, item.display));
            }
            let num_list = choices
                .iter()
                .map(|c| c.key.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            (
                "You are a precise, low-latency scoring system. Evaluate the input state and criteria, then respond with the single score number.",
                opts,
                format!("Select the best score from [{}]:", num_list),
            )
        }
        LabelScheme::Letters => {
            let mut opts = String::new();
            for (idx, item) in choices.iter().enumerate() {
                let letter = LETTER_LABELS.get(idx).copied().unwrap_or("?");
                opts.push_str(&format!("\n{letter}) {}", item.display));
            }
            let letter_list = choices
                .iter()
                .enumerate()
                .map(|(idx, _)| LETTER_LABELS.get(idx).copied().unwrap_or("?"))
                .collect::<Vec<_>>()
                .join(", ");
            (
                "You are a precise, low-latency classification system. Evaluate the input state and criteria, then respond with ONLY the single letter of the best option.",
                opts,
                format!("Select the best option from [{}]:", letter_list),
            )
        }
    };

    let instr = instruction.unwrap_or("Analyze the state and choose the single best option.");

    match template {
        TemplateStyle::Llama3 => format!(
            "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n{}\n<|eot_id|><|start_header_id|>user<|end_header_id|>\n\nInput State:\n\"\"\"\n{}\n\"\"\"\n\nTask: {}\n\nAvailable Options:{}\n\n{}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\nAnswer: ",
            system_msg, state_str, instr, formatted_options, selection_prompt
        ),
        TemplateStyle::ChatML => format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\nInput State:\n\"\"\"\n{}\n\"\"\"\n\nTask: {}\n\nAvailable Options:{}\n\n{}<|im_end|>\n<|im_start|>assistant\nAnswer: ",
            system_msg, state_str, instr, formatted_options, selection_prompt
        ),
        TemplateStyle::Gemma => format!(
            "<start_of_turn>user\n{}\n\nInput State:\n\"\"\"\n{}\n\"\"\"\n\nTask: {}\n\nAvailable Options:{}\n\n{}<end_of_turn>\n<start_of_turn>model\nAnswer: ",
            system_msg, state_str, instr, formatted_options, selection_prompt
        ),
        TemplateStyle::Gemma4 => format!(
            "<|turn>system\n{}<turn|>\n<|turn>user\nInput State:\n\"\"\"\n{}\n\"\"\"\n\nTask: {}\n\nAvailable Options:{}\n\n{}<turn|>\n<|turn>model\n<|thought|>\n<thought|>\nAnswer: ",
            system_msg, state_str, instr, formatted_options, selection_prompt
        ),
    }
}

pub(crate) fn format_generation_prompt(template: TemplateStyle, prompt: &str) -> String {
    let system_msg = "You are a helpful, accurate Bible study assistant. When asked for JSON, respond with valid JSON only.";
    match template {
        TemplateStyle::Llama3 => format!(
            "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>user<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n",
            system_msg, prompt
        ),
        TemplateStyle::ChatML => format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            system_msg, prompt
        ),
        TemplateStyle::Gemma => format!(
            "<start_of_turn>user\n{}\n\n{}<end_of_turn>\n<start_of_turn>model\n",
            system_msg, prompt
        ),
        TemplateStyle::Gemma4 => format!(
            "<|turn>system\n{}<turn|>\n<|turn>user\n{}<turn|>\n<|turn>model\n<|thought|>\n<thought|>\n",
            system_msg, prompt
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_decision_prompt_uses_letters_a_through_z() {
        let choices: Vec<ChoiceItem> = (0..26)
            .map(|i| ChoiceItem {
                key: format!("opt_{i}"),
                display: format!("Option {i}"),
            })
            .collect();
        let prompt = format_decision_prompt(
            TemplateStyle::Llama3,
            None,
            "state",
            &choices,
            LabelScheme::Letters,
        );
        assert!(prompt.contains("\nZ) Option 25"), "{prompt}");
        assert!(prompt.contains(", Z]"), "{prompt}");
        assert!(!prompt.contains("?)"), "{prompt}");
    }

    #[test]
    fn format_decision_prompt_honors_boolean_and_numeric_schemes() {
        let bool_choices = vec![
            ChoiceItem {
                key: "true".into(),
                display: "Yes".into(),
            },
            ChoiceItem {
                key: "false".into(),
                display: "No".into(),
            },
        ];
        let bool_prompt = format_decision_prompt(
            TemplateStyle::ChatML,
            None,
            "state",
            &bool_choices,
            LabelScheme::Boolean,
        );
        assert!(bool_prompt.contains("- true: Yes"), "{bool_prompt}");
        assert!(
            bool_prompt.contains("Select true or false:"),
            "{bool_prompt}"
        );

        let num_choices = vec![
            ChoiceItem {
                key: "0".into(),
                display: "None".into(),
            },
            ChoiceItem {
                key: "1".into(),
                display: "One".into(),
            },
        ];
        let num_prompt = format_decision_prompt(
            TemplateStyle::ChatML,
            None,
            "state",
            &num_choices,
            LabelScheme::Numeric,
        );
        assert!(num_prompt.contains("\n0) None"), "{num_prompt}");
        assert!(
            num_prompt.contains("Select the best score from [0, 1]:"),
            "{num_prompt}"
        );

        let fallback_prompt = format_decision_prompt(
            TemplateStyle::ChatML,
            None,
            "state",
            &num_choices,
            LabelScheme::Letters,
        );
        assert!(fallback_prompt.contains("\nA) None"), "{fallback_prompt}");
        assert!(fallback_prompt.contains("\nB) One"), "{fallback_prompt}");
        assert!(
            fallback_prompt.contains("Select the best option from [A, B]:"),
            "{fallback_prompt}"
        );
    }
}

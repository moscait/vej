//! Prompt templates for decision scoring and text generation.

use llama_cpp_2::model::AddBos;

use crate::models::{TemplateStyle, find_model_spec};

use super::types::ChoiceItem;

const LETTERS: [&str; 16] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P",
];

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
) -> String {
    let is_boolean = choices.len() == 2
        && choices.iter().any(|c| c.key.eq_ignore_ascii_case("true"))
        && choices.iter().any(|c| c.key.eq_ignore_ascii_case("false"));

    let is_numeric = !is_boolean && choices.iter().all(|c| c.key.parse::<u32>().is_ok());

    let (system_msg, formatted_options, selection_prompt) = if is_boolean {
        let mut opts = String::new();
        for item in choices {
            opts.push_str(&format!("\n- {}: {}", item.key, item.display));
        }
        (
            "You are a precise, low-latency decision system. Evaluate the input state and criteria, then respond with true or false.",
            opts,
            "Select true or false:".to_string(),
        )
    } else if is_numeric {
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
    } else {
        let mut opts = String::new();
        for (idx, item) in choices.iter().enumerate() {
            let letter = LETTERS.get(idx).copied().unwrap_or("?");
            opts.push_str(&format!("\n{letter}) {}", item.display));
        }
        let letter_list = choices
            .iter()
            .enumerate()
            .map(|(idx, _)| LETTERS.get(idx).copied().unwrap_or("?"))
            .collect::<Vec<_>>()
            .join(", ");
        (
            "You are a precise, low-latency classification system. Evaluate the input state and criteria, then respond with ONLY the single letter of the best option.",
            opts,
            format!("Select the best option from [{}]:", letter_list),
        )
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

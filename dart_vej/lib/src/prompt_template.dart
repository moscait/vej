/// Enumeration of prompt template styles matching vej's native engine.
///
/// Deprecated: the vej server builds prompts itself, and this copy of its templates
/// drifts from the server's (which now neutralizes special tokens and picks labels per
/// tokenizer). Send requests with [VejClient] instead.
@Deprecated('Prompts are built by the vej server; removed in 0.4.0')
enum TemplateStyle {
  llama3,
  chatml,
  gemma,
  gemma4,
}

/// Helper for constructing identical assistant turn primed prompts.
///
/// Deprecated: see [TemplateStyle].
@Deprecated('Prompts are built by the vej server; removed in 0.4.0')
class PromptBuilder {
  static const List<String> letters = [
    'A',
    'B',
    'C',
    'D',
    'E',
    'F',
    'G',
    'H',
    'I',
    'J',
    'K',
    'L',
    'M',
    'N',
    'O',
    'P'
  ];

  /// Builds a decision prompt string with assistant turn priming ('Answer: ').
  static String buildPrompt({
    required TemplateStyle template,
    required String state,
    required List<MapEntry<String, String>> choices,
    String? instruction,
  }) {
    final isBoolean = choices.length == 2 &&
        choices.any((c) => c.key.toLowerCase() == 'true') &&
        choices.any((c) => c.key.toLowerCase() == 'false');

    final isNumeric =
        !isBoolean && choices.every((c) => int.tryParse(c.key) != null);

    String systemMsg;
    String formattedOptions;
    String selectionPrompt;

    if (isBoolean) {
      final buffer = StringBuffer();
      for (final item in choices) {
        buffer.write('\n- ${item.key}: ${item.value}');
      }
      systemMsg =
          'You are a precise, low-latency decision system. Evaluate the input state and criteria, then respond with true or false.';
      formattedOptions = buffer.toString();
      selectionPrompt = 'Select true or false:';
    } else if (isNumeric) {
      final buffer = StringBuffer();
      for (final item in choices) {
        buffer.write('\n${item.key}) ${item.value}');
      }
      final numList = choices.map((c) => c.key).join(', ');
      systemMsg =
          'You are a precise, low-latency scoring system. Evaluate the input state and criteria, then respond with the single score number.';
      formattedOptions = buffer.toString();
      selectionPrompt = 'Select the best score from [$numList]:';
    } else {
      final buffer = StringBuffer();
      for (var i = 0; i < choices.length; i++) {
        final letter = i < letters.length ? letters[i] : '?';
        buffer.write('\n$letter) ${choices[i].value}');
      }
      final letterList = List.generate(
        choices.length,
        (i) => i < letters.length ? letters[i] : '?',
      ).join(', ');
      systemMsg =
          'You are a precise, low-latency classification system. Evaluate the input state and criteria, then respond with ONLY the single letter of the best option.';
      formattedOptions = buffer.toString();
      selectionPrompt = 'Select the best option from [$letterList]:';
    }

    final taskInstr =
        instruction ?? 'Analyze the state and choose the single best option.';

    switch (template) {
      case TemplateStyle.llama3:
        return '<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n$systemMsg\n<|eot_id|><|start_header_id|>user<|end_header_id|>\n\nInput State:\n"""\n$state\n"""\n\nTask: $taskInstr\n\nAvailable Options:$formattedOptions\n\n$selectionPrompt<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\nAnswer: ';
      case TemplateStyle.chatml:
        return '<|im_start|>system\n$systemMsg<|im_end|>\n<|im_start|>user\nInput State:\n"""\n$state\n"""\n\nTask: $taskInstr\n\nAvailable Options:$formattedOptions\n\n$selectionPrompt<|im_end|>\n<|im_start|>assistant\nAnswer: ';
      case TemplateStyle.gemma:
        return '<start_of_turn>user\n$systemMsg\n\nInput State:\n"""\n$state\n"""\n\nTask: $taskInstr\n\nAvailable Options:$formattedOptions\n\n$selectionPrompt<end_of_turn>\n<start_of_turn>model\nAnswer: ';
      case TemplateStyle.gemma4:
        return '<|turn>system\n$systemMsg<turn|>\n<|turn>user\nInput State:\n"""\n$state\n"""\n\nTask: $taskInstr\n\nAvailable Options:$formattedOptions\n\n$selectionPrompt<turn|>\n<|turn>model\nAnswer: ';
    }
  }
}

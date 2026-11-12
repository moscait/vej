/// Specification of a question for multi-question or schema-driven evaluation.
class QuestionSpec {
  /// The question primitive type: 'choice', 'noul', or 'score'.
  final String type;

  /// Optional instruction for this question (either a String or a Map of instructions/context).
  final dynamic instructions;

  /// Mapping or list of criteria / choices / options.
  final dynamic criteria;

  const QuestionSpec({
    this.type = 'choice',
    this.instructions,
    this.criteria,
  });

  /// Factory helper for creating a categorical 'choice' question.
  factory QuestionSpec.choice({
    dynamic instructions,
    required dynamic criteria,
  }) {
    return QuestionSpec(
      type: 'choice',
      instructions: instructions,
      criteria: criteria,
    );
  }

  /// Factory helper for creating a boolean 'noul' question (Yes / No probability).
  factory QuestionSpec.noul({
    dynamic instructions,
  }) {
    return QuestionSpec(
      type: 'noul',
      instructions: instructions,
    );
  }

  /// Factory helper for creating a continuous expected-value 'score' question.
  factory QuestionSpec.score({
    dynamic instructions,
    dynamic criteria,
  }) {
    return QuestionSpec(
      type: 'score',
      instructions: instructions,
      criteria: criteria,
    );
  }

  Map<String, dynamic> toJson() {
    final map = <String, dynamic>{'type': type};
    if (instructions != null) map['instructions'] = instructions;
    if (criteria != null) map['criteria'] = criteria;
    return map;
  }

  factory QuestionSpec.fromJson(Map<String, dynamic> json) {
    return QuestionSpec(
      type: json['type'] as String? ?? 'choice',
      instructions: json['instructions'],
      criteria: json['criteria'] ?? json['options'] ?? json['choices'] ?? json['levels'],
    );
  }
}

/// Request payload sent to the vej engine.
class DecisionRequest {
  /// Optional target model override (e.g. 'llama-3.2-1b', 'llama-3.2-3b', 'gemma-4-e2b').
  final String? model;

  /// The input state to evaluate. Can be a String, a Map, a List, etc.
  final dynamic state;

  /// Flat list of choices (for flat mode).
  final List<String>? choices;

  /// Instruction or task directive (for flat mode).
  final String? instruction;

  /// Multi-question specifications map (for structured Jev mode).
  final Map<String, QuestionSpec>? questions;

  /// Temperature parameter (defaults to 1.0).
  final double temperature;

  const DecisionRequest({
    this.model,
    required this.state,
    this.choices,
    this.instruction,
    this.questions,
    this.temperature = 1.0,
  });

  Map<String, dynamic> toJson() {
    final map = <String, dynamic>{
      'state': state,
      'temperature': temperature,
    };
    if (model != null) map['model'] = model;
    if (choices != null) map['choices'] = choices;
    if (instruction != null) map['instruction'] = instruction;
    if (questions != null) {
      map['questions'] = questions!.map((k, v) => MapEntry(k, v.toJson()));
    }
    return map;
  }
}

/// Result of a flat decision evaluation.
class DecisionResult {
  /// The chosen key or label.
  final String decision;

  /// Calibrated confidence score (0.0 to 1.0) derived via restricted softmax.
  final double confidence;

  /// Full normalized probability distribution across all candidates.
  final Map<String, double> probabilities;

  /// The model that performed the inference.
  final String model;

  /// Total inference latency in milliseconds.
  final double latencyMs;

  /// Number of tokens evaluated in the prefill pass.
  final int tokensEvaluated;

  const DecisionResult({
    required this.decision,
    required this.confidence,
    required this.probabilities,
    required this.model,
    required this.latencyMs,
    required this.tokensEvaluated,
  });

  factory DecisionResult.fromJson(Map<String, dynamic> json) {
    final rawProbs = json['probabilities'] as Map<String, dynamic>? ?? {};
    final probs = rawProbs.map((k, v) => MapEntry(k, (v as num).toDouble()));

    return DecisionResult(
      decision: json['decision']?.toString() ?? '',
      confidence: (json['confidence'] as num?)?.toDouble() ?? 0.0,
      probabilities: probs,
      model: json['model']?.toString() ?? '',
      latencyMs: (json['latency_ms'] as num?)?.toDouble() ?? 0.0,
      tokensEvaluated: (json['tokens_evaluated'] as num?)?.toInt() ?? 0,
    );
  }

  Map<String, dynamic> toJson() => {
        'decision': decision,
        'confidence': confidence,
        'probabilities': probabilities,
        'model': model,
        'latency_ms': latencyMs,
        'tokens_evaluated': tokensEvaluated,
      };

  @override
  String toString() =>
      'DecisionResult(decision: $decision, confidence: ${(confidence * 100).toStringAsFixed(1)}%, latency: ${latencyMs.toStringAsFixed(1)}ms)';
}

/// Answer output for an individual question in multi-question Jev mode.
class AnswerOutput {
  /// The primitive type: 'choice', 'noul', or 'score'.
  final String type;

  /// Display string or label for the choice (if choice or score).
  final String? choice;

  /// Probability of boolean 'true' (if noul).
  final double? noul;

  /// Continuous expected value score (if score).
  final double? score;

  /// Full normalized probability distribution.
  final Map<String, double> probabilities;

  /// Calibrated confidence (0.0 to 1.0).
  final double confidence;

  const AnswerOutput({
    required this.type,
    this.choice,
    this.noul,
    this.score,
    required this.probabilities,
    required this.confidence,
  });

  factory AnswerOutput.fromJson(Map<String, dynamic> json) {
    final rawProbs = json['probabilities'] as Map<String, dynamic>? ?? {};
    final probs = rawProbs.map((k, v) => MapEntry(k, (v as num).toDouble()));

    return AnswerOutput(
      type: json['type']?.toString() ?? 'choice',
      choice: json['choice']?.toString(),
      noul: (json['noul'] as num?)?.toDouble(),
      score: (json['score'] as num?)?.toDouble(),
      probabilities: probs,
      confidence: (json['confidence'] as num?)?.toDouble() ?? 0.0,
    );
  }

  Map<String, dynamic> toJson() => {
        'type': type,
        if (choice != null) 'choice': choice,
        if (noul != null) 'noul': noul,
        if (score != null) 'score': score,
        'probabilities': probabilities,
        'confidence': confidence,
      };

  @override
  String toString() {
    if (type == 'noul') {
      return 'AnswerOutput(noul: $noul, confidence: ${(confidence * 100).toStringAsFixed(1)}%)';
    } else if (type == 'score') {
      return 'AnswerOutput(score: $score, confidence: ${(confidence * 100).toStringAsFixed(1)}%)';
    }
    return 'AnswerOutput(choice: $choice, confidence: ${(confidence * 100).toStringAsFixed(1)}%)';
  }
}

/// Usage information (token counts).
class UsageInfo {
  final int inputTokens;
  final int outputTokens;

  const UsageInfo({
    required this.inputTokens,
    required this.outputTokens,
  });

  factory UsageInfo.fromJson(Map<String, dynamic> json) {
    return UsageInfo(
      inputTokens: (json['input_tokens'] as num?)?.toInt() ?? 0,
      outputTokens: (json['output_tokens'] as num?)?.toInt() ?? 0,
    );
  }

  Map<String, dynamic> toJson() => {
        'input_tokens': inputTokens,
        'output_tokens': outputTokens,
      };
}

/// Result of a multi-question evaluation in Jev format.
class JevResult {
  /// Name or ID of the model used.
  final String model;

  /// Map of question IDs to their corresponding [AnswerOutput].
  final Map<String, AnswerOutput> answers;

  /// Token usage metadata.
  final UsageInfo usage;

  /// Overall latency in milliseconds.
  final double? latencyMs;

  const JevResult({
    required this.model,
    required this.answers,
    required this.usage,
    this.latencyMs,
  });

  factory JevResult.fromJson(Map<String, dynamic> json) {
    final rawAnswers = json['answers'] as Map<String, dynamic>? ?? {};
    final answers = rawAnswers.map(
      (k, v) => MapEntry(k, AnswerOutput.fromJson(v as Map<String, dynamic>)),
    );

    return JevResult(
      model: json['model']?.toString() ?? '',
      answers: answers,
      usage: UsageInfo.fromJson(json['usage'] as Map<String, dynamic>? ?? {}),
      latencyMs: (json['latency_ms'] as num?)?.toDouble(),
    );
  }

  Map<String, dynamic> toJson() => {
        'model': model,
        'answers': answers.map((k, v) => MapEntry(k, v.toJson())),
        'usage': usage.toJson(),
        if (latencyMs != null) 'latency_ms': latencyMs,
      };
}

/// Model information item returned by the catalog endpoint.
class ModelInfo {
  final String id;
  final String name;
  final String vendor;
  final String country;
  final double sizeGb;
  final List<String> aliases;
  final bool isDownloaded;
  final bool isActive;

  const ModelInfo({
    required this.id,
    required this.name,
    required this.vendor,
    required this.country,
    required this.sizeGb,
    required this.aliases,
    required this.isDownloaded,
    required this.isActive,
  });

  factory ModelInfo.fromJson(Map<String, dynamic> json) {
    return ModelInfo(
      id: json['id'] as String,
      name: json['name'] as String,
      vendor: json['vendor'] as String,
      country: json['country'] as String,
      sizeGb: (json['size_gb'] as num).toDouble(),
      aliases: (json['aliases'] as List<dynamic>?)?.map((e) => e.toString()).toList() ?? [],
      isDownloaded: json['is_downloaded'] as bool? ?? false,
      isActive: json['is_active'] as bool? ?? false,
    );
  }
}

/// Status response from the daemon /health endpoint.
class DaemonStatus {
  final String status;
  final String activeModel;
  final String version;

  const DaemonStatus({
    required this.status,
    required this.activeModel,
    required this.version,
  });

  factory DaemonStatus.fromJson(Map<String, dynamic> json) {
    return DaemonStatus(
      status: json['status'] as String? ?? 'unknown',
      activeModel: json['active_model'] as String? ?? '',
      version: json['version'] as String? ?? '',
    );
  }
}


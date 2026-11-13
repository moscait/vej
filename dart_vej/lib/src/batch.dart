import 'json_reading.dart';
import 'models.dart';

/// A single item within a [BatchDecisionRequest].
class BatchItem {
  /// Unique identifier for this item (e.g. verse reference or ID).
  final String id;

  /// The input state to evaluate.
  final dynamic state;

  /// Optional item-specific choices override.
  final List<String>? choices;

  /// Optional item-specific instruction override.
  final String? instruction;

  /// Optional item-specific questions override.
  final Map<String, QuestionSpec>? questions;

  const BatchItem({
    required this.id,
    required this.state,
    this.choices,
    this.instruction,
    this.questions,
  });

  Map<String, dynamic> toJson() {
    final map = <String, dynamic>{
      'id': id,
      'state': state,
    };
    if (choices != null) map['choices'] = choices;
    if (instruction != null) map['instruction'] = instruction;
    if (questions != null) {
      map['questions'] = questions!.map((k, v) => MapEntry(k, v.toJson()));
    }
    return map;
  }

  factory BatchItem.fromJson(Map<String, dynamic> json) {
    Map<String, QuestionSpec>? q;
    if (json['questions'] is Map) {
      q = (json['questions'] as Map<String, dynamic>).map(
        (k, v) => MapEntry(k, QuestionSpec.fromJson(v as Map<String, dynamic>)),
      );
    }
    return BatchItem(
      id: json['id']?.toString() ?? '',
      state: json['state'],
      choices: (json['choices'] as List?)?.map((e) => e.toString()).toList(),
      instruction: json['instruction']?.toString(),
      questions: q,
    );
  }
}

/// Request payload sent to evaluate multiple items in a single HTTP call.
class BatchDecisionRequest {
  /// Optional target model override.
  final String? model;

  /// Shared choices applied to all items that do not specify their own choices.
  final List<String>? choices;

  /// Shared instruction applied to all items that do not specify their own instruction.
  final String? instruction;

  /// Shared questions applied to all items that do not specify their own questions.
  final Map<String, QuestionSpec>? questions;

  /// The items to evaluate.
  final List<BatchItem> items;

  /// Temperature parameter (defaults to 1.0).
  final double temperature;

  const BatchDecisionRequest({
    this.model,
    this.choices,
    this.instruction,
    this.questions,
    required this.items,
    this.temperature = 1.0,
  });

  Map<String, dynamic> toJson() {
    final map = <String, dynamic>{
      'items': items.map((e) => e.toJson()).toList(),
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

/// Result of evaluating an individual item in a batch request.
class BatchItemResult {
  /// Item identifier matching [BatchItem.id].
  final String id;

  /// The evaluated result: a [JevResult] or a [DecisionResult].
  final EvaluationResult result;

  const BatchItemResult({
    required this.id,
    required this.result,
  });

  /// Convenience getter if [result] is a [JevResult].
  JevResult? get jevResult => switch (result) {
        JevResult r => r,
        DecisionResult() => null,
      };

  /// Convenience getter if [result] is a [DecisionResult].
  DecisionResult? get decisionResult => switch (result) {
        DecisionResult r => r,
        JevResult() => null,
      };

  /// Throws [FormatException] when a field has the wrong type.
  factory BatchItemResult.fromJson(Map<String, dynamic> json) {
    final id = json['id']?.toString() ?? '';
    final EvaluationResult res = json.containsKey('answers')
        ? JevResult.fromJson(json)
        : DecisionResult.fromJson(json);
    return BatchItemResult(id: id, result: res);
  }
}

/// Complete response from a batch evaluation request.
class BatchDecisionResult {
  /// Name or ID of the model used.
  final String model;

  /// Evaluated results for each item in the batch.
  final List<BatchItemResult> results;

  /// Total tokens evaluated across all items.
  final int totalTokens;

  /// Total server latency in milliseconds.
  final double latencyMs;

  const BatchDecisionResult({
    required this.model,
    required this.results,
    required this.totalTokens,
    required this.latencyMs,
  });

  /// Throws [FormatException] when a field has the wrong type.
  factory BatchDecisionResult.fromJson(Map<String, dynamic> json) {
    final items = optList(json, 'results')
        .map((e) => BatchItemResult.fromJson(asObject(e, 'batch result item')))
        .toList();

    return BatchDecisionResult(
      model: json['model']?.toString() ?? '',
      results: items,
      totalTokens: optInt(json, 'total_tokens') ?? 0,
      latencyMs: optDouble(json, 'latency_ms') ?? 0.0,
    );
  }
}

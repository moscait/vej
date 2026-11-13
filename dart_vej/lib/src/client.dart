import 'dart:convert';
import 'package:http/http.dart' as http;

import 'exceptions.dart';
import 'batch.dart';
import 'models.dart';

/// Idiomatic client for vej (and hosted Jev) System 1 AI decision services.
///
/// Works seamlessly across Flutter (Android, iOS, macOS, Windows, Linux, Web) and pure Dart.
///
/// Example:
/// ```dart
/// final vej = VejClient();
///
/// // Simple decision:
/// final res = await vej.decide(
///   state: 'I want a full refund for my broken item',
///   choices: ['refund', 'support', 'shipping'],
/// );
/// print('Decision: ${res.decision} (${res.confidence * 100}%)');
/// ```
class VejClient {
  /// Base URL of the vej daemon or hosted Jev service.
  final Uri baseUrl;

  /// Optional Authorization header (e.g. Bearer token for hosted Jev).
  final String? apiKey;

  /// Underlying HTTP client.
  final http.Client _httpClient;

  /// Timeout duration for inference requests.
  final Duration timeout;

  /// Creates a new [VejClient].
  ///
  /// Defaults to `http://127.0.0.1:8080` (the standard vej daemon endpoint).
  VejClient({
    String baseUrl = 'http://127.0.0.1:8080',
    this.apiKey,
    this.timeout = const Duration(seconds: 30),
    http.Client? httpClient,
  })  : baseUrl = Uri.parse(baseUrl.endsWith('/')
            ? baseUrl.substring(0, baseUrl.length - 1)
            : baseUrl),
        _httpClient = httpClient ?? http.Client();

  /// Creates a client targeting the hosted TypeSafe AI Jev endpoint.
  ///
  /// [baseUrl] points at another Jev-compatible endpoint.
  factory VejClient.hosted({
    required String apiKey,
    String baseUrl = 'https://api.typesafe.ai',
    Duration timeout = const Duration(seconds: 30),
    http.Client? httpClient,
  }) {
    return VejClient(
      baseUrl: baseUrl,
      apiKey: apiKey,
      timeout: timeout,
      httpClient: httpClient,
    );
  }

  /// Closes the internal HTTP client resources.
  void close() {
    _httpClient.close();
  }

  Map<String, String> _buildHeaders() {
    final headers = {
      'Content-Type': 'application/json',
      'Accept': 'application/json',
    };
    if (apiKey != null && apiKey!.isNotEmpty) {
      headers['Authorization'] = 'Bearer $apiKey';
    }
    return headers;
  }

  /// Checks the daemon health and retrieves currently loaded model information.
  Future<DaemonStatus> getStatus() async {
    final uri = baseUrl.replace(path: '${baseUrl.path}/health');
    try {
      final response =
          await _httpClient.get(uri, headers: _buildHeaders()).timeout(timeout);
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = jsonDecode(response.body) as Map<String, dynamic>;
        return DaemonStatus.fromJson(decoded);
      }
      throw VejException(
        'Failed to get daemon health: ${response.body}',
        statusCode: response.statusCode,
      );
    } catch (e) {
      if (e is VejException) rethrow;
      throw VejConnectionException(
        'Could not connect to vej daemon at $uri: $e',
        details: e,
      );
    }
  }

  /// Lists all models configured in the vej catalog and their download status.
  Future<List<ModelInfo>> listModels() async {
    final uri = baseUrl.replace(path: '${baseUrl.path}/v1/models');
    try {
      final response =
          await _httpClient.get(uri, headers: _buildHeaders()).timeout(timeout);
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = jsonDecode(response.body) as List<dynamic>;
        return decoded
            .map((e) => ModelInfo.fromJson(e as Map<String, dynamic>))
            .toList();
      }
      throw VejException(
        'Failed to list models: ${response.body}',
        statusCode: response.statusCode,
      );
    } catch (e) {
      if (e is VejException) rethrow;
      throw VejConnectionException('Failed to list models: $e', details: e);
    }
  }

  /// Hot-switches the active model in the daemon without restarting the server.
  Future<String> switchModel(String modelId) async {
    final uri = baseUrl.replace(path: '${baseUrl.path}/v1/models/switch');
    try {
      final response = await _httpClient
          .post(
            uri,
            headers: _buildHeaders(),
            body: jsonEncode({'model': modelId}),
          )
          .timeout(timeout);

      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = jsonDecode(response.body) as Map<String, dynamic>;
        return decoded['switched_to']?.toString() ?? modelId;
      }
      throw VejException(
        'Failed to switch model to $modelId: ${response.body}',
        statusCode: response.statusCode,
      );
    } catch (e) {
      if (e is VejException) rethrow;
      throw VejConnectionException('Error switching model: $e', details: e);
    }
  }

  /// Evaluates a single-prompt categorical decision (Flat choices format).
  ///
  /// [state] is the subject text or JSON data being evaluated.
  /// [choices] is the candidate options to select from.
  /// [instruction] provides optional classification guidance.
  /// [model] allows per-request model override.
  Future<DecisionResult> decide({
    required dynamic state,
    required List<String> choices,
    String? instruction,
    String? model,
    double temperature = 1.0,
  }) async {
    if (choices.isEmpty) {
      throw const VejValidationException('Choices list cannot be empty.');
    }

    final req = DecisionRequest(
      model: model,
      state: state,
      choices: choices,
      instruction: instruction,
      temperature: temperature,
    );

    final res = await evaluate(req);
    if (res is DecisionResult) {
      return res;
    }
    throw const VejException(
        'Expected flat DecisionResult, received JevResult.');
  }

  /// Evaluates a boolean 'noul' question (Yes / No probability).
  ///
  /// Returns a map entry with the boolean verdict and calibrated confidence.
  Future<({bool verdict, double probability})> decideNoul({
    required dynamic state,
    required String question,
    String? model,
    double temperature = 1.0,
  }) async {
    final jevRes = await evaluateQuestions(
      state: state,
      questions: {
        'noul_check': QuestionSpec.noul(instructions: question),
      },
      model: model,
      temperature: temperature,
    );

    final answer = jevRes.answers['noul_check'];
    if (answer == null) {
      throw const VejException('Failed to get answer for noul check.');
    }

    final trueProb = answer.noul ?? (answer.probabilities['true'] ?? 0.0);
    return (verdict: trueProb >= 0.5, probability: trueProb);
  }

  /// Evaluates a continuous expected-value score question.
  ///
  /// Returns the computed expected value score (e.g. 0.0 to 5.0).
  Future<double> decideScore({
    required dynamic state,
    required String instruction,
    dynamic criteria,
    String? model,
    double temperature = 1.0,
  }) async {
    final jevRes = await evaluateQuestions(
      state: state,
      questions: {
        'score_check': QuestionSpec.score(
          instructions: instruction,
          criteria: criteria,
        ),
      },
      model: model,
      temperature: temperature,
    );

    final answer = jevRes.answers['score_check'];
    if (answer == null || answer.score == null) {
      throw const VejException('Failed to compute expected value score.');
    }

    return answer.score!;
  }

  /// Evaluates multiple questions in a single forward pass / unified schema.
  Future<JevResult> evaluateQuestions({
    required dynamic state,
    required Map<String, QuestionSpec> questions,
    String? model,
    double temperature = 1.0,
  }) async {
    if (questions.isEmpty) {
      throw const VejValidationException('Questions map cannot be empty.');
    }

    final req = DecisionRequest(
      model: model,
      state: state,
      questions: questions,
      temperature: temperature,
    );

    final res = await evaluate(req);
    if (res is JevResult) {
      return res;
    }
    throw const VejException(
        'Expected JevResult, received flat DecisionResult.');
  }

  /// Generic evaluation endpoint supporting either flat [DecisionResult] or multi-question [JevResult].
  Future<dynamic> evaluate(DecisionRequest request) async {
    final uri = baseUrl.replace(path: '${baseUrl.path}/v1/systemone');

    try {
      final response = await _httpClient
          .post(
            uri,
            headers: _buildHeaders(),
            body: jsonEncode(request.toJson()),
          )
          .timeout(timeout);

      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = jsonDecode(response.body) as Map<String, dynamic>;

        // Differentiate between JevResponse (has 'answers') and flat DecisionResponse (has 'decision')
        if (decoded.containsKey('answers')) {
          return JevResult.fromJson(decoded);
        } else if (decoded.containsKey('decision')) {
          return DecisionResult.fromJson(decoded);
        }
        throw VejException('Unrecognized response format: ${response.body}');
      }

      throw VejException(
        'Server returned error: ${response.body}',
        statusCode: response.statusCode,
      );
    } catch (e) {
      if (e is VejException) rethrow;
      throw VejConnectionException(
        'Connection error during decision evaluation: $e',
        details: e,
      );
    }
  }

  /// Evaluates the same [questions] for every item in one HTTP call.
  ///
  /// An empty [items] list returns an empty result without contacting the server.
  Future<BatchDecisionResult> evaluateQuestionsBatch({
    required Map<String, QuestionSpec> questions,
    required List<BatchItem> items,
    String? model,
    double temperature = 1.0,
  }) async {
    if (questions.isEmpty) {
      throw const VejValidationException('Questions map cannot be empty.');
    }
    if (items.isEmpty) {
      return const BatchDecisionResult(
        model: '',
        results: [],
        totalTokens: 0,
        latencyMs: 0.0,
      );
    }
    return evaluateBatch(BatchDecisionRequest(
      model: model,
      questions: questions,
      items: items,
      temperature: temperature,
    ));
  }

  /// Sends a batch request to `/v1/systemone/batch`.
  Future<BatchDecisionResult> evaluateBatch(
      BatchDecisionRequest request) async {
    final uri = baseUrl.replace(path: '${baseUrl.path}/v1/systemone/batch');
    try {
      final response = await _httpClient
          .post(
            uri,
            headers: _buildHeaders(),
            body: jsonEncode(request.toJson()),
          )
          .timeout(timeout);
      if (response.statusCode >= 200 && response.statusCode < 300) {
        final decoded = jsonDecode(response.body) as Map<String, dynamic>;
        return BatchDecisionResult.fromJson(decoded);
      }
      throw VejException(
        'Server returned error: ${response.body}',
        statusCode: response.statusCode,
      );
    } catch (e) {
      if (e is VejException) rethrow;
      throw VejConnectionException(
        'Connection error during batch decision evaluation: $e',
        details: e,
      );
    }
  }
}

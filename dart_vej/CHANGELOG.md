## 0.3.0

Breaking changes:

- `evaluate()` returns `Future<EvaluationResult>` instead of `Future<dynamic>`.
  `EvaluationResult` is sealed (`DecisionResult` or `JevResult`), so `switch`
  over it is exhaustive. `BatchItemResult.result` is now an `EvaluationResult`;
  the `jevResult` and `decisionResult` getters are unchanged.
- A 2xx response that isn't valid JSON, or has the wrong shape, now throws
  `VejException('Invalid response…')` instead of `VejConnectionException`.
  `VejConnectionException` now means only a timeout or a network failure.

Other changes:

- Deprecated `PromptBuilder` and `TemplateStyle`; they duplicate the server's
  prompt templates and will be removed in 0.4.0.
- Shared request handling across all client methods. Error messages for
  connection failures now name the URL, and timeouts say "timed out".

## 0.2.0

- Batch evaluation: `evaluateQuestionsBatch()` and `evaluateBatch()` with `BatchItem`,
  `BatchDecisionRequest` and `BatchDecisionResult` (`POST /v1/systemone/batch`).
- The bearer token (`apiKey`) is now sent on `GET` requests too (`getStatus()`,
  `listModels()`), as vej's remote API requires.
- `VejClient.hosted()` accepts a `baseUrl` for other Jev-compatible endpoints.
- Score criteria keyed `0..n` are sent as a list in level order; other maps are
  sent unchanged.

## 0.1.0

- Initial release of the `vej` Dart and Flutter client library.
- Support for `evaluate()`, System-1 unified schema, and TypeSafe AI proxying.
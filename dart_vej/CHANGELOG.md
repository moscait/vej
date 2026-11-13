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
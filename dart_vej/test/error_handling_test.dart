import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:test/test.dart';
import 'package:vej/vej.dart';

/// A client whose every request is answered by [respond].
VejClient clientReturning(
  Future<http.Response> Function(http.Request request) respond, {
  Duration timeout = const Duration(seconds: 5),
}) {
  return VejClient(httpClient: MockClient(respond), timeout: timeout);
}

VejClient clientWithBody(Object? body, {int status = 200}) {
  final text = body is String ? body : jsonEncode(body);
  return clientReturning((_) async => http.Response(text, status));
}

/// Matches a [VejException] that is not a connection failure.
Matcher invalidResponse() => isA<VejException>()
    .having((e) => e, 'type', isNot(isA<VejConnectionException>()))
    .having((e) => e.message, 'message', contains('Invalid response'));

const decisionJson = {
  'decision': 'refund',
  'confidence': 0.9,
  'probabilities': {'refund': 0.9, 'support': 0.1},
  'model': 'llama-3.2-1b',
  'latency_ms': 12.5,
  'tokens_evaluated': 40,
};

const jevJson = {
  'model': 'llama-3.2-1b',
  'answers': {
    'check': {
      'type': 'noul',
      'noul': 0.8,
      'probabilities': {'true': 0.8, 'false': 0.2},
      'confidence': 0.8,
    },
  },
  'usage': {'input_tokens': 10, 'output_tokens': 0},
};

final flatRequest = DecisionRequest(state: 'x', choices: ['refund', 'support']);

void main() {
  group('evaluate returns a sealed EvaluationResult', () {
    String describe(EvaluationResult result) => switch (result) {
          DecisionResult(:final decision) => 'flat:$decision',
          JevResult(:final answers) => 'jev:${answers.keys.join(',')}',
        };

    test('flat response is a DecisionResult', () async {
      final result = await clientWithBody(decisionJson).evaluate(flatRequest);
      expect(describe(result), 'flat:refund');
    });

    test('Jev response is a JevResult', () async {
      final result = await clientWithBody(jevJson).evaluate(flatRequest);
      expect(describe(result), 'jev:check');
    });

    test('a response that is neither is reported as invalid', () async {
      expect(
          clientWithBody({'unexpected': true}).evaluate(flatRequest),
          throwsA(isA<VejException>()
              .having((e) => e, 'type', isNot(isA<VejConnectionException>()))));
    });

    test('batch item results are typed', () async {
      final batch = await clientWithBody({
        'model': 'm',
        'results': [
          {'id': 'a', ...decisionJson},
        ],
        'total_tokens': 40,
        'latency_ms': 12.5,
      }).evaluateBatch(BatchDecisionRequest(
        choices: ['refund', 'support'],
        items: [BatchItem(id: 'a', state: 'x')],
      ));
      final EvaluationResult result = batch.results.single.result;
      expect(result, isA<DecisionResult>());
      expect(batch.results.single.decisionResult?.decision, 'refund');
    });
  });

  group('malformed responses are VejException, not connection errors', () {
    test('body that is not JSON', () {
      expect(clientWithBody('<html>proxy error</html>').evaluate(flatRequest),
          throwsA(invalidResponse()));
    });

    test('probabilities with a non-numeric value', () {
      final bad = {
        ...decisionJson,
        'probabilities': {'refund': 'high'},
      };
      expect(clientWithBody(bad).evaluate(flatRequest),
          throwsA(invalidResponse()));
    });

    test('answers that are not objects', () {
      final bad = {
        ...jevJson,
        'answers': {'check': 'yes'},
      };
      expect(clientWithBody(bad).evaluate(flatRequest),
          throwsA(invalidResponse()));
    });

    test('batch results that are not a list', () {
      expect(
        clientWithBody({'model': 'm', 'results': 'nope'}).evaluateBatch(
          BatchDecisionRequest(
            choices: ['a', 'b'],
            items: [BatchItem(id: 'a', state: 'x')],
          ),
        ),
        throwsA(invalidResponse()),
      );
    });

    test('health response that is a list', () {
      expect(clientWithBody([]).getStatus(), throwsA(invalidResponse()));
    });

    test('model list that is an object', () {
      expect(clientWithBody({'models': []}).listModels(),
          throwsA(invalidResponse()));
    });

    test('model entry missing its id', () {
      expect(
        clientWithBody([
          {'name': 'x'}
        ]).listModels(),
        throwsA(invalidResponse()),
      );
    });
  });

  group('transport failures are VejConnectionException', () {
    test('timeout', () {
      final client = clientReturning(
        (_) => Future.delayed(
          const Duration(milliseconds: 300),
          () => http.Response(jsonEncode(decisionJson), 200),
        ),
        timeout: const Duration(milliseconds: 20),
      );
      expect(
        client.evaluate(flatRequest),
        throwsA(isA<VejConnectionException>()
            .having((e) => e.message, 'message', contains('timed out'))),
      );
    });

    test('network error from the HTTP client', () {
      final client = clientReturning((request) async =>
          throw http.ClientException('Connection refused', request.url));
      expect(client.getStatus(), throwsA(isA<VejConnectionException>()));
      expect(
          client.evaluate(flatRequest), throwsA(isA<VejConnectionException>()));
    });
  });

  group('HTTP errors keep their status code', () {
    test('400 from the server', () {
      expect(
        clientWithBody('Choices list cannot be empty', status: 400)
            .evaluate(flatRequest),
        throwsA(isA<VejException>()
            .having((e) => e.statusCode, 'statusCode', 400)
            .having((e) => e, 'type', isNot(isA<VejConnectionException>()))
            .having((e) => e.message, 'message',
                contains('Choices list cannot be empty'))),
      );
    });

    test('switchModel failure names the model', () {
      expect(
        clientWithBody('not downloaded', status: 500)
            .switchModel('llama-3.1-8b'),
        throwsA(isA<VejException>()
            .having((e) => e.message, 'message', contains('llama-3.1-8b'))),
      );
    });
  });
}

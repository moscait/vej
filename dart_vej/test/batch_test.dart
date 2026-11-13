import 'dart:convert';
import 'package:test/test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:vej/vej.dart';

void main() {
  group('VejClient Batch API', () {
    test('evaluateQuestionsBatch serializes request and parses response',
        () async {
      final mockClient = MockClient((request) async {
        expect(request.url.path, '/v1/systemone/batch');
        final decoded = jsonDecode(request.body) as Map<String, dynamic>;
        expect(decoded['questions'], isNotNull);
        expect(decoded['items'], hasLength(2));

        return http.Response(
          jsonEncode({
            'model': 'test-model',
            'results': [
              {
                'id': 'item-1',
                'answers': {
                  'check': {
                    'type': 'noul',
                    'noul': 0.95,
                    'probabilities': {'true': 0.95, 'false': 0.05},
                    'confidence': 0.95,
                  },
                },
                'usage': {'input_tokens': 50, 'output_tokens': 0},
                'latency_ms': 5.0,
              },
              {
                'id': 'item-2',
                'answers': {
                  'check': {
                    'type': 'noul',
                    'noul': 0.12,
                    'probabilities': {'true': 0.12, 'false': 0.88},
                    'confidence': 0.88,
                  },
                },
                'usage': {'input_tokens': 45, 'output_tokens': 0},
                'latency_ms': 4.5,
              },
            ],
            'total_tokens': 95,
            'latency_ms': 9.5,
          }),
          200,
          headers: {'content-type': 'application/json'},
        );
      });

      final client = VejClient(
        baseUrl: 'http://127.0.0.1:47831',
        httpClient: mockClient,
      );

      final result = await client.evaluateQuestionsBatch(
        questions: {
          'check': QuestionSpec.noul(instructions: 'Is this true?'),
        },
        items: const [
          BatchItem(id: 'item-1', state: 'State 1'),
          BatchItem(id: 'item-2', state: 'State 2'),
        ],
      );

      expect(result.model, 'test-model');
      expect(result.results, hasLength(2));
      expect(result.totalTokens, 95);
      expect(result.latencyMs, 9.5);

      final first = result.results[0];
      expect(first.id, 'item-1');
      expect(first.jevResult, isNotNull);
      expect(first.jevResult!.answers['check']?.noul, 0.95);

      final second = result.results[1];
      expect(second.id, 'item-2');
      expect(second.jevResult, isNotNull);
      expect(second.jevResult!.answers['check']?.noul, 0.12);
    });
    test(
        'evaluateQuestionsBatch with no items returns at once without a request',
        () async {
      var requests = 0;
      final client = VejClient(
        httpClient: MockClient((_) async {
          requests++;
          return http.Response('{}', 200);
        }),
      );

      final result = await client.evaluateQuestionsBatch(
        questions: {'check': QuestionSpec.noul(instructions: 'Is this true?')},
        items: const [],
      );

      expect(result.results, isEmpty);
      expect(requests, 0);
    });

    test('evaluateQuestionsBatch rejects an empty questions map', () {
      final client = VejClient(
        httpClient: MockClient((_) async => http.Response('{}', 200)),
      );
      expect(
        client.evaluateQuestionsBatch(
          questions: const {},
          items: const [BatchItem(id: '1', state: 'x')],
        ),
        throwsA(isA<VejValidationException>()),
      );
    });

    test('evaluateBatch sends shared settings and item overrides', () async {
      late Map<String, dynamic> sent;
      final client = VejClient(
        apiKey: 'vej_token',
        httpClient: MockClient((request) async {
          expect(request.headers['Authorization'], 'Bearer vej_token');
          sent = jsonDecode(request.body) as Map<String, dynamic>;
          return http.Response(
            jsonEncode({
              'model': 'm',
              'results': [
                {'id': 'a', 'decision': 'refund', 'confidence': 0.9},
              ],
            }),
            200,
          );
        }),
      );

      final result = await client.evaluateBatch(const BatchDecisionRequest(
        model: 'gemma-4-e2b',
        choices: ['refund', 'shipping'],
        items: [
          BatchItem(id: 'a', state: 'charged twice', instruction: 'Triage'),
        ],
        temperature: 0.5,
      ));

      expect(sent['model'], 'gemma-4-e2b');
      expect(sent['choices'], ['refund', 'shipping']);
      expect(sent['temperature'], 0.5);
      expect(sent['items'], [
        {'id': 'a', 'state': 'charged twice', 'instruction': 'Triage'},
      ]);
      expect(result.results.single.decisionResult?.decision, 'refund');
    });

    test('a failed batch surfaces the status code', () async {
      final client = VejClient(
        httpClient: MockClient((_) async => http.Response('nope', 401)),
      );
      expect(
        client.evaluateBatch(const BatchDecisionRequest(
          items: [BatchItem(id: 'a', state: 'x')],
          choices: ['a', 'b'],
        )),
        throwsA(
            isA<VejException>().having((e) => e.statusCode, 'statusCode', 401)),
      );
    });
  });
}

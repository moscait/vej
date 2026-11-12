import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:test/test.dart';
import 'package:vej/vej.dart';

void main() {
  group('Bearer token on every request', () {
    late List<http.Request> requests;
    late VejClient client;

    setUp(() {
      requests = [];
      client = VejClient(
        apiKey: 'vej_token',
        httpClient: MockClient((request) async {
          requests.add(request);
          final body = request.url.path == '/v1/models'
              ? '[]'
              : jsonEncode({'status': 'ok', 'active_model': 'm'});
          return http.Response(body, 200);
        }),
      );
    });

    test('getStatus sends the token (vej remote API requires it on GET)',
        () async {
      await client.getStatus();
      expect(requests.single.headers['Authorization'], 'Bearer vej_token');
    });

    test('listModels sends the token', () async {
      await client.listModels();
      expect(requests.single.headers['Authorization'], 'Bearer vej_token');
    });

    test('no Authorization header without a token', () async {
      final anonymous = VejClient(
        httpClient: MockClient((request) async {
          expect(request.headers.containsKey('Authorization'), isFalse);
          return http.Response('{"status":"ok"}', 200);
        }),
      );
      await anonymous.getStatus();
    });
  });

  group('VejClient.hosted', () {
    test('defaults to the TypeSafe AI endpoint', () {
      final client = VejClient.hosted(apiKey: 'k');
      expect(client.baseUrl.toString(), 'https://api.typesafe.ai');
    });

    test('accepts another endpoint', () {
      final client = VejClient.hosted(
        apiKey: 'k',
        baseUrl: 'https://jev.example.com/',
      );
      expect(client.baseUrl.toString(), 'https://jev.example.com');
      expect(client.apiKey, 'k');
    });
  });

  group('QuestionSpec.score criteria', () {
    test('levels keyed 0..n are sent as a list in numeric order', () {
      final spec = QuestionSpec.score(criteria: {
        '2': 'strong',
        '0': 'none',
        '1': 'weak',
      });
      expect(spec.toJson()['criteria'], ['none', 'weak', 'strong']);
    });

    test('other maps are sent unchanged, so no level is renumbered', () {
      final criteria = {'0': 'safe', '5': 'catastrophic'};
      expect(QuestionSpec.score(criteria: criteria).toJson()['criteria'],
          criteria);
      final labels = {'low': 'a', 'high': 'b'};
      expect(QuestionSpec.score(criteria: labels).toJson()['criteria'], labels);
    });

    test('choice criteria are never converted', () {
      final criteria = {'0': 'a', '1': 'b'};
      expect(QuestionSpec.choice(criteria: criteria).toJson()['criteria'],
          criteria);
    });
  });
}

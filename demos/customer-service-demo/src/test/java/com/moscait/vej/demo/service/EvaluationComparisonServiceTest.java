package com.moscait.vej.demo.service;

import static org.junit.jupiter.api.Assertions.*;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.moscait.vej.VejClient;
import com.moscait.vej.demo.domain.CallCategory;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class EvaluationComparisonServiceTest {

    private static final ObjectMapper MAPPER = new ObjectMapper();

    private HttpServer vejServer;
    private HttpServer jevServer;
    private String vejUrl;
    private String jevUrl;
    private EvaluationComparisonService service;

    @BeforeEach
    void setUp() throws IOException {
        vejServer = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        vejServer.start();
        vejUrl = "http://127.0.0.1:" + vejServer.getAddress().getPort();

        jevServer = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        jevServer.start();
        jevUrl = "http://127.0.0.1:" + jevServer.getAddress().getPort();

        service = new EvaluationComparisonService(
                Duration.ofSeconds(5),
                "jev-latest",
                (baseUrl, apiKey, timeout) -> VejClient.builder()
                        .baseUrl(baseUrl)
                        .apiKey(apiKey)
                        .timeout(timeout)
                        .build());
    }

    @AfterEach
    void tearDown() {
        vejServer.stop(0);
        jevServer.stop(0);
    }

    @Test
    void comparesVejAndJevWhenTokenProvided() throws Exception {
        AtomicReference<JsonNode> vejBody = new AtomicReference<>();
        AtomicReference<JsonNode> jevBody = new AtomicReference<>();
        AtomicReference<String> jevAuth = new AtomicReference<>();

        vejServer.createContext("/v1/systemone", ex -> {
            vejBody.set(MAPPER.readTree(ex.getRequestBody().readAllBytes()));
            byte[] out = """
                    {
                      "model": "llama-3.2-3b",
                      "answers": {
                        "category": {
                          "type": "choice",
                          "choice": "refund",
                          "confidence": 0.88,
                          "probabilities": {"refund": 0.88, "complaint": 0.08, "support": 0.04}
                        },
                        "urgency": {
                          "type": "score",
                          "choice": "4",
                          "score": 3.9,
                          "probabilities": {"3": 0.2, "4": 0.7, "5": 0.1},
                          "confidence": 0.7
                        },
                        "escalation_needed": {
                          "type": "noul",
                          "noul": 0.62,
                          "probabilities": {"true": 0.62, "false": 0.38},
                          "confidence": 0.62
                        }
                      },
                      "usage": {"input_tokens": 184, "output_tokens": 0},
                      "latency_ms": 92.4
                    }
                    """.getBytes(StandardCharsets.UTF_8);
            ex.sendResponseHeaders(200, out.length);
            ex.getResponseBody().write(out);
            ex.close();
        });

        jevServer.createContext("/v1/systemone", ex -> {
            jevBody.set(MAPPER.readTree(ex.getRequestBody().readAllBytes()));
            jevAuth.set(ex.getRequestHeaders().getFirst("Authorization"));
            byte[] out = """
                    {
                      "model": "jev-1.13.0",
                      "answers": {
                        "category": {
                          "type": "choice",
                          "choice": "refund",
                          "confidence": 1.0,
                          "probabilities": {"refund": 1.0, "complaint": 0.0}
                        },
                        "urgency": {
                          "type": "score",
                          "choice": "4",
                          "score": 4.0,
                          "probabilities": {"4": 1.0},
                          "confidence": 1.0
                        },
                        "escalation_needed": {
                          "type": "noul",
                          "noul": 0.85,
                          "probabilities": {"true": 0.85, "false": 0.15},
                          "confidence": 0.85
                        }
                      },
                      "usage": {"input_tokens": 412, "output_tokens": 36}
                    }
                    """.getBytes(StandardCharsets.UTF_8);
            ex.sendResponseHeaders(200, out.length);
            ex.getResponseBody().write(out);
            ex.close();
        });

        var pair = service.compare(
                "I was double charged and need a refund.",
                vejUrl,
                "llama-3.2-3b",
                jevUrl,
                "secret-jev-token",
                "jev-latest",
                true,
                1.0);

        assertTrue(pair.vejOutcome().isSuccess());
        assertEquals(CallCategory.REFUND, pair.vejOutcome().predictedCategory());
        assertEquals(0.88, pair.vejOutcome().categoryConfidence(), 1e-6);
        assertEquals(3.9, pair.vejOutcome().urgencyScore(), 1e-6);
        assertEquals(Boolean.TRUE, pair.vejOutcome().escalationNeeded());
        assertEquals(92.4, pair.vejOutcome().serverLatencyMs(), 1e-6);
        assertEquals(0, pair.vejOutcome().outputTokens());

        assertTrue(pair.jevOutcome().isSuccess());
        assertEquals("Bearer secret-jev-token", jevAuth.get());
        assertEquals(CallCategory.REFUND, pair.jevOutcome().predictedCategory());
        assertEquals(36, pair.jevOutcome().outputTokens());
        assertNull(pair.jevOutcome().serverLatencyMs());

        JsonNode sentQuestions = vejBody.get().get("questions");
        assertTrue(sentQuestions.has("category"));
        assertTrue(sentQuestions.has("urgency"));
        assertTrue(sentQuestions.has("escalation_needed"));
        assertTrue(sentQuestions.get("urgency").get("criteria").isArray(),
                "score criteria must be serialized as a JSON array for Jev & vej compatibility");

        JsonNode sentJev = jevBody.get();
        assertFalse(sentJev.has("temperature"), "Hosted Jev payload should omit temperature");
        assertTrue(sentJev.get("questions").get("urgency").get("criteria").isArray());
    }

    @Test
    void skipsJevWhenTokenMissingAndIsolatesHttpErrors() {
        vejServer.createContext("/v1/systemone", ex -> {
            byte[] out = """
                    {
                      "model": "llama-3.2-1b",
                      "answers": {
                        "category": {
                          "type": "choice",
                          "choice": "support",
                          "confidence": 0.77,
                          "probabilities": {"support": 0.77}
                        }
                      },
                      "usage": {"input_tokens": 110, "output_tokens": 0},
                      "latency_ms": 51.0
                    }
                    """.getBytes(StandardCharsets.UTF_8);
            ex.sendResponseHeaders(200, out.length);
            ex.getResponseBody().write(out);
            ex.close();
        });

        jevServer.createContext("/v1/systemone", ex -> {
            byte[] out = """
                    {"detail":{"error_type":"authentication_error","message":"Cannot authenticate with the server."}}
                    """.getBytes(StandardCharsets.UTF_8);
            ex.sendResponseHeaders(401, out.length);
            ex.getResponseBody().write(out);
            ex.close();
        });

        var skippedPair = service.compare(
                "Help resetting MFA.", vejUrl, null, jevUrl, "", null, false, 1.0);
        assertTrue(skippedPair.vejOutcome().isSuccess());
        assertEquals(CallCategory.SUPPORT, skippedPair.vejOutcome().predictedCategory());
        assertTrue(skippedPair.jevOutcome().skipped());

        var errorPair = service.compare(
                "Help resetting MFA.", vejUrl, null, jevUrl + "/v1/systemone", "\"Bearer bad-token\"", null, false, 1.0);
        assertTrue(errorPair.vejOutcome().isSuccess());
        assertFalse(errorPair.jevOutcome().isSuccess());
        assertTrue(errorPair.jevOutcome().errorMessage().contains("401"));
        assertTrue(errorPair.jevOutcome().errorMessage().contains("Cannot authenticate with the server."));
        assertFalse(errorPair.jevOutcome().errorMessage().contains("HTTP 401: HTTP 401:"));

        var connErrPair = service.compare(
                "Help resetting MFA.", vejUrl, null, "http://127.0.0.1:1", "tok", null, false, 1.0);
        assertFalse(connErrPair.jevOutcome().isSuccess());
        assertNotNull(connErrPair.jevOutcome().errorMessage());
        assertFalse(connErrPair.jevOutcome().errorMessage().contains("NullPointerException"));

        var envFallbackService = new EvaluationComparisonService(
                Duration.ofSeconds(5),
                "jev-latest",
                "\"Bearer env-fallback-token\"",
                (baseUrl, apiKey, timeout) -> VejClient.builder()
                        .baseUrl(baseUrl)
                        .apiKey(apiKey)
                        .timeout(timeout)
                        .build());
        assertEquals("env-fallback-token", envFallbackService.configuredJevApiKey());
        var fallbackPair = envFallbackService.compare(
                "Help resetting MFA.", vejUrl, null, jevUrl, "", null, false, 1.0);
        assertFalse(fallbackPair.jevOutcome().skipped());
        assertTrue(fallbackPair.jevOutcome().errorMessage().contains("401"));
    }
}

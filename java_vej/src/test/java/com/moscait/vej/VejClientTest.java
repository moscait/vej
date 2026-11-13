package com.moscait.vej;

import static org.junit.jupiter.api.Assertions.*;

import com.fasterxml.jackson.databind.JsonNode;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletionException;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

/** Exercises the client against an in-process fake daemon returning canned vej responses. */
class VejClientTest {

    private HttpServer server;
    private VejClient client;
    private final AtomicReference<JsonNode> lastBody = new AtomicReference<>();
    private final AtomicReference<String> lastAuth = new AtomicReference<>();

    private void route(String path, int status, String response) {
        server.createContext(path, ex -> {
            byte[] in = ex.getRequestBody().readAllBytes();
            lastBody.set(in.length == 0 ? null : VejClient.MAPPER.readTree(in));
            lastAuth.set(ex.getRequestHeaders().getFirst("Authorization"));
            byte[] out = response.getBytes(StandardCharsets.UTF_8);
            ex.sendResponseHeaders(status, out.length);
            ex.getResponseBody().write(out);
            ex.close();
        });
    }

    @BeforeEach
    void start() throws IOException {
        server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.start();
        client = VejClient.builder()
                .baseUrl("http://127.0.0.1:" + server.getAddress().getPort() + "/")
                .timeout(Duration.ofSeconds(5))
                .build();
    }

    @AfterEach
    void stop() {
        server.stop(0);
    }

    @Test
    void decideSendsFlatRequestAndParsesResult() {
        route("/v1/systemone", 200, """
                {"decision":"refund","confidence":0.91,
                 "probabilities":{"refund":0.91,"support":0.06,"billing":0.03},
                 "model":"llama-3.2-1b","latency_ms":182.5,"tokens_evaluated":97}""");

        DecisionResult r = client.decide("charged twice", List.of("refund", "support", "billing"), "Route it");

        assertEquals("refund", r.decision());
        assertEquals(0.91, r.confidence(), 1e-9);
        assertEquals(0.06, r.probabilities().get("support"), 1e-9);
        assertEquals(182.5, r.latencyMs(), 1e-9);
        assertEquals(97, r.tokensEvaluated());

        JsonNode sent = lastBody.get();
        assertEquals("charged twice", sent.get("state").asText());
        assertEquals(3, sent.get("choices").size());
        assertEquals("Route it", sent.get("instruction").asText());
        assertEquals(1.0, sent.get("temperature").asDouble());
        assertFalse(sent.has("model"), "null fields are omitted");
        assertFalse(sent.has("questions"));
    }

    @Test
    void structuredStateIsSentAsJson() {
        route("/v1/systemone", 200, """
                {"decision":"a","confidence":1,"probabilities":{"a":1},"model":"m","latency_ms":1,"tokens_evaluated":1}""");

        record Ticket(String ticketId, String text) {}
        client.decide(new Ticket("T-1", "hi"), List.of("a", "b"));

        JsonNode state = lastBody.get().get("state");
        assertEquals("T-1", state.get("ticket_id").asText());
        assertEquals("hi", state.get("text").asText());
    }

    @Test
    void questionsModeParsesAllAnswerTypes() {
        route("/v1/systemone", 200, """
                {"model":"gemma-4-e2b",
                 "answers":{
                   "engineering_depth":{"type":"score","choice":"4","score":3.8,
                                        "probabilities":{"3":0.3,"4":0.6,"5":0.1},"confidence":0.6},
                   "talent_profile":{"type":"choice","choice":"systems_builder",
                                     "probabilities":{"systems_builder":0.8,"ml_practitioner":0.2},"confidence":0.8},
                   "immediate_interview":{"type":"noul","noul":0.72,
                                          "probabilities":{"true":0.72,"false":0.28},"confidence":0.72}},
                 "usage":{"input_tokens":410,"output_tokens":3},"latency_ms":350.0}""");

        EvaluationResult res = client.evaluate(VejPresets.talentProfileScreening("resume", null));
        JevResult jev = assertInstanceOf(JevResult.class, res);

        assertEquals(List.of("engineering_depth", "talent_profile", "immediate_interview"),
                List.copyOf(jev.answers().keySet()), "answer order is preserved");
        assertEquals(3.8, jev.answer("engineering_depth").score(), 1e-9);
        assertEquals("systems_builder", jev.answer("talent_profile").choice());
        assertNull(jev.answer("talent_profile").noul());
        assertEquals(0.72, jev.answer("immediate_interview").noul(), 1e-9);
        assertEquals(410, jev.usage().inputTokens());
        assertEquals(350.0, jev.latencyMs(), 1e-9);

        JsonNode q = lastBody.get().get("questions");
        assertEquals("score", q.get("engineering_depth").get("type").asText());
        assertEquals("noul", q.get("immediate_interview").get("type").asText());
        assertFalse(q.get("immediate_interview").has("criteria"));
        assertEquals("engineering_depth", q.fieldNames().next(), "question order is preserved");
    }

    @Test
    void decideNoulAndScore() {
        route("/v1/systemone", 200, """
                {"model":"m","answers":{
                   "noul_check":{"type":"noul","noul":0.31,"probabilities":{"true":0.31,"false":0.69},"confidence":0.69},
                   "score_check":{"type":"score","score":2.5,"probabilities":{},"confidence":0.4}},
                 "usage":{"input_tokens":1,"output_tokens":1}}""");

        NoulResult noul = client.decideNoul("text", "Is this spam?");
        assertFalse(noul.verdict());
        assertEquals(0.31, noul.probability(), 1e-9);

        assertEquals(2.5, client.decideScore("text", "Rate it", Map.of("0", "bad", "5", "good")), 1e-9);
    }

    @Test
    void batchParsesMixedItems() {
        route("/v1/systemone/batch", 200, """
                {"model":"llama-3.2-1b","total_tokens":300,"latency_ms":410.2,
                 "results":[
                   {"id":"t1","decision":"refund","confidence":0.9,"probabilities":{"refund":0.9,"billing":0.1},
                    "model":"llama-3.2-1b","latency_ms":200,"tokens_evaluated":150},
                   {"id":"t2","decision":"billing","confidence":0.7,"probabilities":{"refund":0.3,"billing":0.7},
                    "model":"llama-3.2-1b","latency_ms":210,"tokens_evaluated":150}]}""");

        BatchResult b = client.evaluateBatch(BatchRequest.builder()
                .choices("refund", "billing")
                .item("t1", "refund me")
                .item("t2", "invoice wrong")
                .build());

        assertEquals(2, b.results().size());
        assertEquals(300, b.totalTokens());
        DecisionResult t2 = assertInstanceOf(DecisionResult.class, b.result("t2").orElseThrow());
        assertEquals("billing", t2.decision());
        assertEquals("t1", lastBody.get().get("items").get(0).get("id").asText());
    }

    @Test
    void asyncEvaluate() {
        route("/v1/systemone", 200, """
                {"decision":"a","confidence":0.5,"probabilities":{"a":0.5,"b":0.5},"model":"m","latency_ms":1,"tokens_evaluated":1}""");

        EvaluationResult r = client.evaluateAsync(
                DecisionRequest.builder().state("s").choices("a", "b").build()).join();
        assertEquals("a", assertInstanceOf(DecisionResult.class, r).decision());
    }

    @Test
    void httpErrorBecomesVejExceptionWithStatus() {
        route("/v1/systemone", 500, "Evaluation error: context overflow");

        VejException e = assertThrows(VejException.class, () -> client.decide("s", List.of("a", "b")));
        assertEquals(500, e.statusCode());
        assertTrue(e.getMessage().contains("context overflow"));
        assertFalse(e instanceof VejConnectionException);
    }

    @Test
    void asyncHttpErrorCompletesExceptionally() {
        route("/v1/systemone", 403, "Origin not allowed");

        CompletionException e = assertThrows(CompletionException.class, () -> client.evaluateAsync(
                DecisionRequest.builder().state("s").choices("a").build()).join());
        VejException cause = assertInstanceOf(VejException.class, e.getCause());
        assertEquals(403, cause.statusCode());
    }

    @Test
    void unreachableDaemonIsConnectionException() {
        server.stop(0);
        assertThrows(VejConnectionException.class, () -> client.status());
    }

    @Test
    void validationHappensBeforeNetwork() {
        assertThrows(VejValidationException.class, () -> client.decide("s", List.of()));
        assertThrows(VejValidationException.class, () -> client.evaluateQuestions("s", Map.of()));
        assertThrows(VejValidationException.class,
                () -> client.evaluate(DecisionRequest.builder().state("s").build()));
        assertThrows(VejValidationException.class,
                () -> client.evaluateBatch(BatchRequest.builder().choices("a").build()));
        assertThrows(VejValidationException.class,
                () -> client.evaluateBatch(BatchRequest.builder().item("x", "s").build()));
        assertNull(lastBody.get());
    }

    @Test
    void statusModelsAndSwitch() {
        route("/health", 200, """
                {"status":"ok","active_model":"gemma-4-e2b","version":"0.1.0"}""");
        route("/v1/models", 200, """
                [{"id":"gemma-4-e2b","name":"Gemma 4 E2B","vendor":"Google","country":"US","size_gb":1.6,
                  "aliases":["gemma4"],"is_downloaded":true,"is_active":true}]""");
        route("/v1/models/switch", 200, """
                {"status":"success","switched_to":"llama-3.2-3b"}""");

        DaemonStatus s = client.status();
        assertTrue(s.isOk());
        assertEquals("gemma-4-e2b", s.activeModel());

        ModelInfo m = client.listModels().get(0);
        assertEquals(1.6, m.sizeGb(), 1e-9);
        assertTrue(m.isDownloaded());
        assertEquals(List.of("gemma4"), m.aliases());

        assertEquals("llama-3.2-3b", client.switchModel("3b"));
        assertEquals("3b", lastBody.get().get("model").asText());
    }

    @Test
    void apiKeyIsSentAsBearer() {
        route("/health", 200, """
                {"status":"ok","active_model":"m","version":"1"}""");
        VejClient authed = VejClient.builder()
                .baseUrl("http://127.0.0.1:" + server.getAddress().getPort())
                .apiKey("secret")
                .build();
        authed.status();
        assertEquals("Bearer secret", lastAuth.get());
    }
}

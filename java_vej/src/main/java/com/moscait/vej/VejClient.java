package com.moscait.vej;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;
import com.fasterxml.jackson.databind.json.JsonMapper;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;

/**
 * Thread-safe client for the vej daemon (and the hosted, Jev-compatible endpoint).
 *
 * <pre>{@code
 * VejClient vej = VejClient.create();   // http://127.0.0.1:8080
 *
 * DecisionResult r = vej.decide("I was charged twice, please refund me",
 *         List.of("refund", "support", "billing"));
 * System.out.printf("%s (%.0f%%)%n", r.decision(), r.confidence() * 100);
 * }</pre>
 *
 * <p>Every blocking method has an {@code *Async} counterpart for the core evaluation calls. All
 * failures are reported as unchecked {@link VejException}s; connection problems and timeouts as
 * {@link VejConnectionException}.
 *
 * <p>One instance should be shared across the application; it holds a pooled {@link HttpClient}.
 */
public final class VejClient {

    /** Default local daemon address. */
    public static final String DEFAULT_BASE_URL = "http://127.0.0.1:8080";
    /** Hosted TypeSafe AI Jev endpoint. */
    public static final String HOSTED_BASE_URL = "https://api.typesafe.ai";

    static final ObjectMapper MAPPER = JsonMapper.builder()
            .propertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE)
            .disable(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES)
            .serializationInclusion(JsonInclude.Include.NON_NULL)
            .build();

    private final String baseUrl;
    private final String apiKey;
    private final Duration timeout;
    private final HttpClient http;

    private VejClient(Builder b) {
        String url = Objects.requireNonNull(b.baseUrl, "baseUrl").trim();
        while (url.endsWith("/")) url = url.substring(0, url.length() - 1);
        this.baseUrl = url;
        this.apiKey = b.apiKey;
        this.timeout = Objects.requireNonNull(b.timeout, "timeout");
        this.http = b.httpClient != null
                ? b.httpClient
                : HttpClient.newBuilder().connectTimeout(Duration.ofSeconds(5)).build();
    }

    /** A client for the local daemon at {@value #DEFAULT_BASE_URL}. */
    public static VejClient create() {
        return builder().build();
    }

    /** A client for a daemon at {@code baseUrl}. */
    public static VejClient create(String baseUrl) {
        return builder().baseUrl(baseUrl).build();
    }

    /**
     * A client for the hosted Jev endpoint. Only {@code /v1/systemone} (the {@code decide*} and
     * {@code evaluate} methods) is available there.
     */
    public static VejClient hosted(String apiKey) {
        return builder().baseUrl(HOSTED_BASE_URL).apiKey(apiKey).build();
    }

    public static Builder builder() {
        return new Builder();
    }

    public String baseUrl() {
        return baseUrl;
    }

    // ---------------------------------------------------------------- daemon management

    /** {@code GET /health}: daemon status and active model. */
    public DaemonStatus status() {
        return convert(send(get("/health")), DaemonStatus.class);
    }

    /** {@code GET /v1/models}: the model catalog with download and active flags. */
    public List<ModelInfo> listModels() {
        return convert(send(get("/v1/models")), new TypeReference<List<ModelInfo>>() {});
    }

    /** {@code POST /v1/models/switch}: hot-switch the active model; returns the resolved model id. */
    public String switchModel(String modelId) {
        requireText(modelId, "modelId");
        JsonNode res = send(post("/v1/models/switch", Map.of("model", modelId)));
        return res.hasNonNull("switched_to") ? res.get("switched_to").asText() : modelId;
    }

    /** {@code POST /v1/generate}: free-form text generation with the active model. */
    public String generate(String prompt, int maxTokens) {
        requireText(prompt, "prompt");
        if (maxTokens <= 0) throw new VejValidationException("maxTokens must be positive.");
        JsonNode res = send(post("/v1/generate", Map.of("prompt", prompt, "max_tokens", maxTokens)));
        return res.path("response").asText("");
    }

    // ---------------------------------------------------------------- decisions

    /** Flat-mode decision: pick one of {@code choices} for {@code state}. */
    public DecisionResult decide(Object state, List<String> choices) {
        return decide(DecisionRequest.builder().state(state).choices(choices).build());
    }

    /** Flat-mode decision with a task instruction. */
    public DecisionResult decide(Object state, List<String> choices, String instruction) {
        return decide(DecisionRequest.builder().state(state).choices(choices).instruction(instruction).build());
    }

    /** Flat-mode decision from a full request (model override, temperature, ...). */
    public DecisionResult decide(DecisionRequest request) {
        if (request.choices() == null || request.choices().isEmpty()) {
            throw new VejValidationException("choices cannot be empty.");
        }
        return expect(evaluate(request), DecisionResult.class);
    }

    /** Yes/no question; the verdict is {@code true} when P(true) &gt;= 0.5. */
    public NoulResult decideNoul(Object state, String question) {
        JevResult res = evaluateQuestions(state, Map.of("noul_check", QuestionSpec.noul(question)));
        AnswerOutput a = res.answer("noul_check");
        if (a == null) throw new VejException("Response has no answer for the noul question.");
        double p = a.noul() != null ? a.noul() : a.probabilities().getOrDefault("true", 0.0);
        return new NoulResult(p >= 0.5, p);
    }

    /**
     * Expected-value score, e.g. 0.0 to 5.0 over levels {@code {"0": "...", ..., "5": "..."}}.
     *
     * @param criteria the score levels (a Map of level to description, or a List)
     */
    public double decideScore(Object state, String instruction, Object criteria) {
        JevResult res = evaluateQuestions(state, Map.of("score_check", QuestionSpec.score(instruction, criteria)));
        AnswerOutput a = res.answer("score_check");
        if (a == null || a.score() == null) throw new VejException("Response has no score.");
        return a.score();
    }

    /** Several questions about one state in a single call. Pass a LinkedHashMap to keep order. */
    public JevResult evaluateQuestions(Object state, Map<String, QuestionSpec> questions) {
        if (questions == null || questions.isEmpty()) {
            throw new VejValidationException("questions cannot be empty.");
        }
        return expect(evaluate(DecisionRequest.builder().state(state).questions(questions).build()), JevResult.class);
    }

    /** {@code POST /v1/systemone}: returns a {@link DecisionResult} or {@link JevResult}. */
    public EvaluationResult evaluate(DecisionRequest request) {
        validate(request);
        return parseEvaluation(send(post("/v1/systemone", request)));
    }

    /** Async {@link #evaluate}. Failures complete the future exceptionally with a {@link VejException}. */
    public CompletableFuture<EvaluationResult> evaluateAsync(DecisionRequest request) {
        try {
            validate(request);
            return sendAsync(post("/v1/systemone", request)).thenApply(VejClient::parseEvaluation);
        } catch (VejException e) {
            return CompletableFuture.failedFuture(e);
        }
    }

    /** {@code POST /v1/systemone/batch}: evaluate many states in one call (local daemon only). */
    public BatchResult evaluateBatch(BatchRequest request) {
        validate(request);
        return parseBatch(send(post("/v1/systemone/batch", request)));
    }

    /** Async {@link #evaluateBatch}. */
    public CompletableFuture<BatchResult> evaluateBatchAsync(BatchRequest request) {
        try {
            validate(request);
            return sendAsync(post("/v1/systemone/batch", request)).thenApply(VejClient::parseBatch);
        } catch (VejException e) {
            return CompletableFuture.failedFuture(e);
        }
    }

    // ---------------------------------------------------------------- validation

    private static void validate(DecisionRequest r) {
        Objects.requireNonNull(r, "request");
        boolean flat = r.choices() != null && !r.choices().isEmpty();
        boolean questions = r.questions() != null && !r.questions().isEmpty();
        if (!flat && !questions) {
            throw new VejValidationException("Request needs non-empty choices or questions.");
        }
    }

    private static void validate(BatchRequest r) {
        Objects.requireNonNull(r, "request");
        if (r.items().isEmpty()) throw new VejValidationException("Batch needs at least one item.");
        boolean shared = (r.choices() != null && !r.choices().isEmpty())
                || (r.questions() != null && !r.questions().isEmpty());
        for (BatchItem item : r.items()) {
            requireText(item.id(), "batch item id");
            boolean own = (item.choices() != null && !item.choices().isEmpty())
                    || (item.questions() != null && !item.questions().isEmpty());
            if (!shared && !own) {
                throw new VejValidationException(
                        "Batch item '" + item.id() + "' has no choices or questions, and the batch sets none.");
            }
        }
    }

    private static void requireText(String s, String name) {
        if (s == null || s.isBlank()) throw new VejValidationException(name + " cannot be empty.");
    }

    // ---------------------------------------------------------------- HTTP

    private HttpRequest get(String path) {
        return base(path).GET().build();
    }

    private HttpRequest post(String path, Object body) {
        String json;
        try {
            json = MAPPER.writeValueAsString(body);
        } catch (JsonProcessingException e) {
            throw new VejValidationException("Request body is not JSON-serializable: " + e.getOriginalMessage());
        }
        return base(path)
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(json))
                .build();
    }

    private HttpRequest.Builder base(String path) {
        HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(baseUrl + path))
                .timeout(timeout)
                .header("Accept", "application/json")
                .header("User-Agent", "vej-java/0.1.0");
        if (apiKey != null && !apiKey.isBlank()) b.header("Authorization", "Bearer " + apiKey);
        return b;
    }

    private JsonNode send(HttpRequest req) {
        HttpResponse<String> res;
        try {
            res = http.send(req, HttpResponse.BodyHandlers.ofString());
        } catch (IOException e) {
            throw connectionError(req, e);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new VejConnectionException("Interrupted while calling " + req.uri(), e);
        }
        return handle(res);
    }

    private CompletableFuture<JsonNode> sendAsync(HttpRequest req) {
        return http.sendAsync(req, HttpResponse.BodyHandlers.ofString())
                .handle((res, err) -> {
                    if (err != null) {
                        Throwable cause = err instanceof CompletionException && err.getCause() != null
                                ? err.getCause() : err;
                        throw cause instanceof VejException ve ? ve : connectionError(req, cause);
                    }
                    return handle(res);
                });
    }

    private static VejConnectionException connectionError(HttpRequest req, Throwable cause) {
        return new VejConnectionException("Could not reach vej at " + req.uri() + ": " + cause, cause);
    }

    private static JsonNode handle(HttpResponse<String> res) {
        String body = res.body();
        int code = res.statusCode();
        if (code < 200 || code >= 300) {
            throw new VejException(errorMessage(body), code, body);
        }
        try {
            return MAPPER.readTree(body);
        } catch (JsonProcessingException e) {
            throw new VejException("Invalid JSON from server: " + e.getOriginalMessage(), code, body, e);
        }
    }

    /** Server errors are plain text, or JSON with "error"/"message". */
    private static String errorMessage(String body) {
        if (body == null || body.isBlank()) return "Request failed";
        try {
            JsonNode n = MAPPER.readTree(body);
            if (n.hasNonNull("error")) return n.get("error").asText();
            if (n.hasNonNull("message")) return n.get("message").asText();
        } catch (JsonProcessingException ignored) {
            // plain-text body
        }
        return body.length() > 500 ? body.substring(0, 500) + "..." : body;
    }

    // ---------------------------------------------------------------- parsing

    static EvaluationResult parseEvaluation(JsonNode n) {
        if (n.has("answers")) return convert(n, JevResult.class);
        if (n.has("decision")) return convert(n, DecisionResult.class);
        throw new VejException("Unrecognized response format: " + n);
    }

    static BatchResult parseBatch(JsonNode n) {
        List<BatchItemResult> results = new ArrayList<>();
        for (Iterator<JsonNode> it = n.path("results").elements(); it.hasNext(); ) {
            JsonNode item = it.next();
            results.add(new BatchItemResult(item.path("id").asText(), parseEvaluation(item)));
        }
        return new BatchResult(
                n.path("model").asText(""),
                results,
                n.path("total_tokens").asLong(0),
                n.path("latency_ms").asDouble(0));
    }

    private static <T> T convert(JsonNode n, Class<T> type) {
        try {
            return MAPPER.treeToValue(n, type);
        } catch (JsonProcessingException | IllegalArgumentException e) {
            throw new VejException("Could not parse " + type.getSimpleName() + ": " + e.getMessage());
        }
    }

    private static <T> T convert(JsonNode n, TypeReference<T> type) {
        try {
            return MAPPER.treeToValue(n, MAPPER.getTypeFactory().constructType(type));
        } catch (JsonProcessingException | IllegalArgumentException e) {
            throw new VejException("Could not parse response: " + e.getMessage());
        }
    }

    private static <T extends EvaluationResult> T expect(EvaluationResult r, Class<T> type) {
        if (type.isInstance(r)) return type.cast(r);
        throw new VejException("Expected " + type.getSimpleName() + " but server returned "
                + r.getClass().getSimpleName() + ".");
    }

    // ---------------------------------------------------------------- builder

    /** Builder for {@link VejClient}. */
    public static final class Builder {
        private String baseUrl = DEFAULT_BASE_URL;
        private String apiKey;
        private Duration timeout = Duration.ofSeconds(30);
        private HttpClient httpClient;

        private Builder() {}

        /** Daemon base URL, default {@value VejClient#DEFAULT_BASE_URL}. */
        public Builder baseUrl(String baseUrl) { this.baseUrl = baseUrl; return this; }

        /** Bearer token sent as {@code Authorization} (needed for the hosted endpoint). */
        public Builder apiKey(String apiKey) { this.apiKey = apiKey; return this; }

        /** Per-request timeout, default 30 s. Large batches or cold model switches may need more. */
        public Builder timeout(Duration timeout) { this.timeout = timeout; return this; }

        /** Custom {@link HttpClient} (proxy, executor, SSL settings). */
        public Builder httpClient(HttpClient httpClient) { this.httpClient = httpClient; return this; }

        public VejClient build() {
            return new VejClient(this);
        }
    }
}

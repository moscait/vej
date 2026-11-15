package com.moscait.vej.demo.service;

import com.moscait.vej.AnswerOutput;
import com.moscait.vej.DecisionRequest;
import com.moscait.vej.EvaluationResult;
import com.moscait.vej.JevResult;
import com.moscait.vej.ModelInfo;
import com.moscait.vej.QuestionSpec;
import com.moscait.vej.VejClient;
import com.moscait.vej.VejException;
import com.moscait.vej.demo.domain.CallCategory;
import com.moscait.vej.demo.domain.EngineOutcome;
import java.time.Duration;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Service;

/**
 * Evaluates customer service interaction text against both local {@code vej} and hosted {@code Jev}
 * using identical {@code questions}-mode payloads and captures high-resolution timing and usage.
 */
@Service
public class EvaluationComparisonService {

    private static final Logger log = LoggerFactory.getLogger(EvaluationComparisonService.class);

    public static final String QUESTION_CATEGORY = "category";
    public static final String QUESTION_URGENCY = "urgency";
    public static final String QUESTION_ESCALATION = "escalation_needed";

    private static final List<String> URGENCY_CRITERIA = List.of(
            "No urgency — purely positive feedback or appreciation",
            "Minimal urgency — calm gratitude or minor FYI comment",
            "Low urgency — routine how-to question or gentle inquiry",
            "Moderate urgency — billing discrepancy or shipping inconvenience",
            "High urgency — repeated failure, blocked workflow, or clear frustration",
            "Critical urgency — outage, financial harm, forgery, or immediate churn threat");

    @FunctionalInterface
    public interface ClientFactory {
        VejClient create(String baseUrl, String apiKey, Duration timeout);
    }

    public record ComparisonPair(EngineOutcome vejOutcome, EngineOutcome jevOutcome) {}

    private final Duration requestTimeout;
    private final String defaultJevModel;
    private final String configuredJevApiKey;
    private final ClientFactory clientFactory;
    private final ExecutorService executor;

    @org.springframework.beans.factory.annotation.Autowired
    public EvaluationComparisonService(
            @Value("${vej.request-timeout-seconds:45}") int requestTimeoutSeconds,
            @Value("${vej.jev-model:jev-latest}") String defaultJevModel,
            @Value("${vej.jev-api-key:}") String configuredJevApiKey) {
        this(
                Duration.ofSeconds(Math.max(5, requestTimeoutSeconds)),
                defaultJevModel,
                configuredJevApiKey,
                (baseUrl, apiKey, timeout) -> {
                    VejClient.Builder builder = VejClient.builder()
                            .baseUrl(baseUrl)
                            .timeout(timeout);
                    if (apiKey != null && !apiKey.isBlank()) {
                        builder.apiKey(apiKey.trim());
                    }
                    return builder.build();
                });
        if (this.configuredJevApiKey.isEmpty()) {
            log.info("No TYPESAFE_API_KEY / JEV_API_KEY found in environment/config; UI will start in vej-only mode until a token is entered.");
        } else {
            log.info("Detected Jev API token from environment/config (length {} chars); Dual Compare ready.",
                    this.configuredJevApiKey.length());
        }
    }

    public EvaluationComparisonService(
            Duration requestTimeout,
            String defaultJevModel,
            ClientFactory clientFactory) {
        this(requestTimeout, defaultJevModel, "", clientFactory);
    }

    public EvaluationComparisonService(
            Duration requestTimeout,
            String defaultJevModel,
            String configuredJevApiKey,
            ClientFactory clientFactory) {
        this.requestTimeout = requestTimeout;
        this.defaultJevModel = (defaultJevModel == null || defaultJevModel.isBlank())
                ? "jev-latest"
                : defaultJevModel.trim();
        this.configuredJevApiKey = normalizeToken(configuredJevApiKey);
        this.clientFactory = clientFactory;
        this.executor = Executors.newCachedThreadPool(r -> {
            Thread t = new Thread(r, "vej-eval-worker");
            t.setDaemon(true);
            return t;
        });
    }

    public String defaultJevModel() {
        return defaultJevModel;
    }

    public String configuredJevApiKey() {
        return configuredJevApiKey;
    }

    public List<ModelInfo> listLocalModels(String vejBaseUrl) {
        return clientFactory.create(vejBaseUrl, null, Duration.ofSeconds(5)).listModels();
    }

    public String switchLocalModel(String vejBaseUrl, String modelId) {
        return clientFactory.create(vejBaseUrl, null, Duration.ofSeconds(60)).switchModel(modelId);
    }

    /**
     * Evaluates one text input concurrently against local {@code vej} and hosted {@code Jev}
     * (when a token is provided).
     */
    public ComparisonPair compare(
            String textToEvaluate,
            String vejBaseUrl,
            String vejModel,
            String jevBaseUrl,
            String jevApiToken,
            String jevModel,
            boolean includeSecondaryQuestions,
            double temperature) {

        Map<String, QuestionSpec> questions = buildQuestions(includeSecondaryQuestions);
        String effectiveVejModel = blankToNull(vejModel);
        String effectiveJevModel = (jevModel == null || jevModel.isBlank()) ? defaultJevModel : jevModel.trim();

        DecisionRequest vejRequest = DecisionRequest.builder()
                .model(effectiveVejModel)
                .state(textToEvaluate)
                .questions(questions)
                .temperature(temperature)
                .build();

        VejClient vejClient = clientFactory.create(vejBaseUrl, null, requestTimeout);
        CompletableFuture<EngineOutcome> vejFuture = CompletableFuture.supplyAsync(
                () -> executeSingle(vejClient, "vej", effectiveVejModel, vejRequest),
                executor);

        CompletableFuture<EngineOutcome> jevFuture;
        String effectiveToken = normalizeToken(jevApiToken);
        if (effectiveToken.isEmpty()) {
            effectiveToken = configuredJevApiKey;
        }
        if (effectiveToken.isEmpty()) {
            jevFuture = CompletableFuture.completedFuture(
                    EngineOutcome.skipped("Jev", "Provide a Jev API token to run hosted comparison"));
        } else {
            String targetJevUrl = normalizeJevBaseUrl(jevBaseUrl);
            DecisionRequest jevRequest = DecisionRequest.builder()
                    .model(effectiveJevModel)
                    .state(textToEvaluate)
                    .questions(questions)
                    .omitTemperature()
                    .build();
            VejClient jevClient = clientFactory.create(targetJevUrl, effectiveToken, requestTimeout);
            jevFuture = CompletableFuture.supplyAsync(
                    () -> executeSingle(jevClient, "Jev", effectiveJevModel, jevRequest),
                    executor);
        }

        return new ComparisonPair(vejFuture.join(), jevFuture.join());
    }

    public static Map<String, QuestionSpec> buildQuestions(boolean includeSecondaryQuestions) {
        Map<String, QuestionSpec> questions = new LinkedHashMap<>();
        questions.put(
                QUESTION_CATEGORY,
                QuestionSpec.choice(
                        "Classify the customer's primary reason for calling into exactly one category.",
                        CallCategory.toCriteriaMap()));
        if (includeSecondaryQuestions) {
            questions.put(
                    QUESTION_URGENCY,
                    QuestionSpec.score(
                            "Rate the interaction urgency and emotional heat on a 0 to 5 scale.",
                            URGENCY_CRITERIA));
            questions.put(
                    QUESTION_ESCALATION,
                    QuestionSpec.noul(
                            "Does this call require immediate supervisor escalation or priority retention handling?"));
        }
        return questions;
    }

    private EngineOutcome executeSingle(
            VejClient client,
            String engineName,
            String requestedModel,
            DecisionRequest request) {
        long startNs = System.nanoTime();
        try {
            EvaluationResult result = client.evaluate(request);
            double elapsedMs = (System.nanoTime() - startNs) / 1_000_000.0;
            if (!(result instanceof JevResult jevResult)) {
                return EngineOutcome.error(
                        engineName,
                        requestedModel,
                        "Unexpected response type: " + result.getClass().getSimpleName(),
                        elapsedMs);
            }
            return toOutcome(engineName, requestedModel, jevResult, elapsedMs);
        } catch (VejException ex) {
            double elapsedMs = (System.nanoTime() - startNs) / 1_000_000.0;
            return EngineOutcome.error(engineName, requestedModel, formatError(ex), elapsedMs);
        } catch (RuntimeException ex) {
            double elapsedMs = (System.nanoTime() - startNs) / 1_000_000.0;
            return EngineOutcome.error(engineName, requestedModel, ex.getMessage(), elapsedMs);
        }
    }

    private static EngineOutcome toOutcome(
            String engineName,
            String requestedModel,
            JevResult result,
            double clientLatencyMs) {
        AnswerOutput categoryAnswer = result.answer(QUESTION_CATEGORY);
        if (categoryAnswer == null) {
            return EngineOutcome.error(
                    engineName,
                    result.model(),
                    "Response missing '" + QUESTION_CATEGORY + "' answer",
                    clientLatencyMs);
        }

        String rawChoice = categoryAnswer.choice();
        if ((rawChoice == null || rawChoice.isBlank())
                && categoryAnswer.probabilities() != null
                && !categoryAnswer.probabilities().isEmpty()) {
            rawChoice = categoryAnswer.probabilities().entrySet().stream()
                    .max(Comparator.comparingDouble(Map.Entry::getValue))
                    .map(Map.Entry::getKey)
                    .orElse(null);
        }
        CallCategory predicted = CallCategory.fromKey(rawChoice).orElse(null);

        Double urgencyScore = null;
        AnswerOutput urgencyAnswer = result.answer(QUESTION_URGENCY);
        if (urgencyAnswer != null) {
            urgencyScore = urgencyAnswer.score();
            if (urgencyScore == null && urgencyAnswer.choice() != null) {
                try {
                    urgencyScore = Double.parseDouble(urgencyAnswer.choice().trim());
                } catch (NumberFormatException ignored) {
                    // leave null
                }
            }
        }

        Double escalationProb = null;
        Boolean escalationNeeded = null;
        AnswerOutput escalationAnswer = result.answer(QUESTION_ESCALATION);
        if (escalationAnswer != null) {
            escalationProb = escalationAnswer.noul();
            if (escalationProb == null && escalationAnswer.probabilities() != null) {
                escalationProb = escalationAnswer.probabilities().get("true");
            }
            if (escalationProb != null) {
                escalationNeeded = escalationProb >= 0.5;
            }
        }

        String modelName = (result.model() != null && !result.model().isBlank())
                ? result.model()
                : requestedModel;

        return new EngineOutcome(
                engineName,
                modelName,
                false,
                null,
                null,
                rawChoice,
                predicted,
                categoryAnswer.confidence(),
                categoryAnswer.probabilities(),
                urgencyScore,
                escalationProb,
                escalationNeeded,
                clientLatencyMs,
                result.latencyMs(),
                result.usage() != null ? result.usage().inputTokens() : 0L,
                result.usage() != null ? result.usage().outputTokens() : 0L);
    }

    private static String formatError(VejException ex) {
        if (ex.getMessage() != null && !ex.getMessage().isBlank()) {
            return ex.getMessage();
        }
        Integer code = ex.statusCode();
        if (code != null && code > 0) {
            return "HTTP " + code;
        }
        return ex.getClass().getSimpleName();
    }

    private static String blankToNull(String s) {
        return (s == null || s.isBlank()) ? null : s.trim();
    }

    static String normalizeToken(String raw) {
        if (raw == null) {
            return "";
        }
        String t = raw.trim();
        if ((t.startsWith("\"") && t.endsWith("\"") && t.length() >= 2)
                || (t.startsWith("'") && t.endsWith("'") && t.length() >= 2)) {
            t = t.substring(1, t.length() - 1).trim();
        }
        if (t.length() >= 7 && t.substring(0, 7).equalsIgnoreCase("Bearer ")) {
            t = t.substring(7).trim();
        }
        return t;
    }

    static String normalizeJevBaseUrl(String rawUrl) {
        if (rawUrl == null || rawUrl.isBlank()) {
            return VejClient.HOSTED_BASE_URL;
        }
        String url = rawUrl.trim();
        while (url.endsWith("/")) {
            url = url.substring(0, url.length() - 1);
        }
        if (url.endsWith("/v1/systemone")) {
            url = url.substring(0, url.length() - "/v1/systemone".length());
        }
        return url.isEmpty() ? VejClient.HOSTED_BASE_URL : url;
    }
}

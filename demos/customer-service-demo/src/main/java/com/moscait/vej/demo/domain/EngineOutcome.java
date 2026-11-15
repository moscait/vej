package com.moscait.vej.demo.domain;

import java.util.Map;
import java.util.Optional;

/**
 * Evaluation output and timing metrics from a single engine ({@code vej} or {@code Jev}).
 */
public record EngineOutcome(
        String engineName,
        String model,
        boolean skipped,
        String skipReason,
        String errorMessage,
        String rawCategoryChoice,
        CallCategory predictedCategory,
        double categoryConfidence,
        Map<String, Double> categoryProbabilities,
        Double urgencyScore,
        Double escalationProbability,
        Boolean escalationNeeded,
        double clientLatencyMs,
        Double serverLatencyMs,
        long inputTokens,
        long outputTokens) {

    public boolean isSuccess() {
        return !skipped && errorMessage == null && rawCategoryChoice != null;
    }

    public Optional<CallCategory> predictedCategoryOpt() {
        return Optional.ofNullable(predictedCategory);
    }

    public static EngineOutcome skipped(String engineName, String reason) {
        return new EngineOutcome(
                engineName, null, true, reason, null,
                null, null, 0.0, Map.of(), null, null, null,
                0.0, null, 0L, 0L);
    }

    public static EngineOutcome error(String engineName, String model, String errorMessage, double clientLatencyMs) {
        return new EngineOutcome(
                engineName, model, false, null, errorMessage,
                null, null, 0.0, Map.of(), null, null, null,
                clientLatencyMs, null, 0L, 0L);
    }
}

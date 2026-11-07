package com.moscait.vej;

import java.util.Map;

/**
 * Flat-mode decision.
 *
 * @param decision        the chosen label
 * @param confidence      calibrated probability (0..1) of {@code decision}
 * @param probabilities   normalized distribution over all candidates
 * @param model           model that performed the inference
 * @param latencyMs       server-side inference latency in milliseconds
 * @param tokensEvaluated tokens in the prefill pass
 */
public record DecisionResult(
        String decision,
        double confidence,
        Map<String, Double> probabilities,
        String model,
        double latencyMs,
        long tokensEvaluated) implements EvaluationResult {

    public DecisionResult {
        probabilities = probabilities == null ? Map.of() : Map.copyOf(probabilities);
    }
}

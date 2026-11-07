package com.moscait.vej;

import java.util.List;
import java.util.Optional;

/**
 * Response from {@code POST /v1/systemone/batch}.
 *
 * @param model       model that performed the inference
 * @param results     per-item results in request order
 * @param totalTokens tokens evaluated across the batch
 * @param latencyMs   server-side latency for the whole batch in milliseconds
 */
public record BatchResult(String model, List<BatchItemResult> results, long totalTokens, double latencyMs) {

    public BatchResult {
        results = results == null ? List.of() : List.copyOf(results);
    }

    /** The result for item {@code id}, if present. */
    public Optional<EvaluationResult> result(String id) {
        return results.stream().filter(r -> r.id().equals(id)).map(BatchItemResult::result).findFirst();
    }
}

package com.moscait.vej;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Question-mode (Jev-format) result.
 *
 * @param model     model that performed the inference
 * @param answers   answers keyed by question id, in response order
 * @param usage     token usage
 * @param latencyMs server-side latency in milliseconds, or {@code null} if not reported
 */
public record JevResult(
        String model,
        Map<String, AnswerOutput> answers,
        UsageInfo usage,
        Double latencyMs) implements EvaluationResult {

    public JevResult {
        answers = answers == null ? Map.of() : Collections.unmodifiableMap(new LinkedHashMap<>(answers));
        usage = usage == null ? new UsageInfo(0, 0) : usage;
    }

    /** The answer for {@code questionId}, or {@code null}. */
    public AnswerOutput answer(String questionId) {
        return answers.get(questionId);
    }
}

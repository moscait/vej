package com.moscait.vej;

import java.util.Map;

/**
 * Answer to one question in a {@link JevResult}.
 *
 * @param type          {@code "choice"}, {@code "noul"} or {@code "score"}
 * @param choice        chosen label (choice / score questions), else {@code null}
 * @param noul          probability of {@code true} (noul questions), else {@code null}
 * @param score         expected-value score (score questions), else {@code null}
 * @param probabilities normalized distribution over the candidates
 * @param confidence    calibrated confidence (0..1)
 */
public record AnswerOutput(
        String type,
        String choice,
        Double noul,
        Double score,
        Map<String, Double> probabilities,
        double confidence) {

    public AnswerOutput {
        probabilities = probabilities == null ? Map.of() : Map.copyOf(probabilities);
    }
}

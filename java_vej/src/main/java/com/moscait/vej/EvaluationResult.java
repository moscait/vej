package com.moscait.vej;

/**
 * Result of {@code POST /v1/systemone}: a {@link DecisionResult} for flat-mode requests or a
 * {@link JevResult} for question-mode requests.
 *
 * <pre>{@code
 * switch (client.evaluate(req)) {
 *     case DecisionResult d -> System.out.println(d.decision());
 *     case JevResult j -> System.out.println(j.answers());
 * }
 * }</pre>
 * (Pattern matching for switch requires Java 21; on 17 use {@code instanceof}.)
 */
public sealed interface EvaluationResult permits DecisionResult, JevResult {
    /** Model that performed the inference. */
    String model();
}

package com.moscait.vej;

/**
 * Result for one {@link BatchItem}.
 *
 * @param id     the item's id
 * @param result a {@link DecisionResult} (flat mode) or {@link JevResult} (question mode)
 */
public record BatchItemResult(String id, EvaluationResult result) {}

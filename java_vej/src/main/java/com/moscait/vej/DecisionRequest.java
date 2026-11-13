package com.moscait.vej;

import com.fasterxml.jackson.annotation.JsonInclude;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Request body for {@code POST /v1/systemone}.
 *
 * <p>Use either flat mode ({@code choices} + optional {@code instruction}), which returns a
 * {@link DecisionResult}, or question mode ({@code questions}), which returns a {@link JevResult}.
 *
 * @param model       optional per-request model override, e.g. {@code "llama-3.2-3b"}
 * @param state       the input to evaluate: a String, Map, List, record or any JSON-serializable object
 * @param choices     flat-mode candidate labels
 * @param instruction flat-mode task directive
 * @param questions   question-mode specs keyed by question id (iteration order is preserved)
 * @param temperature softmax temperature (default 1.0)
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public record DecisionRequest(
        String model,
        Object state,
        List<String> choices,
        String instruction,
        Map<String, QuestionSpec> questions,
        double temperature) {

    public DecisionRequest {
        choices = choices == null ? null : List.copyOf(choices);
        questions = questions == null ? null : java.util.Collections.unmodifiableMap(new LinkedHashMap<>(questions));
    }

    public static Builder builder() {
        return new Builder();
    }

    /** Fluent builder for {@link DecisionRequest}. */
    public static final class Builder {
        private String model;
        private Object state;
        private List<String> choices;
        private String instruction;
        private Map<String, QuestionSpec> questions;
        private double temperature = 1.0;

        private Builder() {}

        public Builder model(String model) { this.model = model; return this; }
        public Builder state(Object state) { this.state = state; return this; }
        public Builder choices(List<String> choices) { this.choices = choices; return this; }
        public Builder choices(String... choices) { this.choices = List.of(choices); return this; }
        public Builder instruction(String instruction) { this.instruction = instruction; return this; }
        public Builder questions(Map<String, QuestionSpec> questions) { this.questions = questions; return this; }
        public Builder temperature(double temperature) { this.temperature = temperature; return this; }

        /** Adds one question, keeping insertion order. */
        public Builder question(String id, QuestionSpec spec) {
            if (questions == null) questions = new LinkedHashMap<>();
            else if (!(questions instanceof LinkedHashMap)) questions = new LinkedHashMap<>(questions);
            questions.put(id, spec);
            return this;
        }

        public DecisionRequest build() {
            return new DecisionRequest(model, state, choices, instruction, questions, temperature);
        }
    }
}

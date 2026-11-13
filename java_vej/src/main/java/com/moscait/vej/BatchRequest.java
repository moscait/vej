package com.moscait.vej;

import com.fasterxml.jackson.annotation.JsonInclude;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Request body for {@code POST /v1/systemone/batch}: many states evaluated in one call against
 * shared {@code choices} or {@code questions} (each item may override them).
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public record BatchRequest(
        String model,
        List<String> choices,
        String instruction,
        Map<String, QuestionSpec> questions,
        List<BatchItem> items,
        double temperature) {

    public BatchRequest {
        items = items == null ? List.of() : List.copyOf(items);
    }

    public static Builder builder() {
        return new Builder();
    }

    /** Fluent builder for {@link BatchRequest}. */
    public static final class Builder {
        private String model;
        private List<String> choices;
        private String instruction;
        private Map<String, QuestionSpec> questions;
        private final List<BatchItem> items = new ArrayList<>();
        private double temperature = 1.0;

        private Builder() {}

        public Builder model(String model) { this.model = model; return this; }
        public Builder choices(List<String> choices) { this.choices = choices; return this; }
        public Builder choices(String... choices) { this.choices = List.of(choices); return this; }
        public Builder instruction(String instruction) { this.instruction = instruction; return this; }
        public Builder questions(Map<String, QuestionSpec> questions) { this.questions = questions; return this; }
        public Builder temperature(double temperature) { this.temperature = temperature; return this; }

        public Builder question(String id, QuestionSpec spec) {
            if (questions == null) questions = new LinkedHashMap<>();
            else if (!(questions instanceof LinkedHashMap)) questions = new LinkedHashMap<>(questions);
            questions.put(id, spec);
            return this;
        }

        public Builder item(BatchItem item) { items.add(item); return this; }
        public Builder item(String id, Object state) { items.add(BatchItem.of(id, state)); return this; }
        public Builder items(List<BatchItem> more) { items.addAll(more); return this; }

        public BatchRequest build() {
            return new BatchRequest(model, choices, instruction, questions, items, temperature);
        }
    }
}

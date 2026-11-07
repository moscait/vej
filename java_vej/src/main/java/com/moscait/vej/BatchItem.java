package com.moscait.vej;

import com.fasterxml.jackson.annotation.JsonInclude;
import java.util.List;
import java.util.Map;

/**
 * One item in a {@link BatchRequest}. Per-item {@code choices}, {@code instruction} and
 * {@code questions} override the batch-level defaults.
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public record BatchItem(
        String id,
        Object state,
        List<String> choices,
        String instruction,
        Map<String, QuestionSpec> questions) {

    /** An item that uses the batch-level choices / questions. */
    public static BatchItem of(String id, Object state) {
        return new BatchItem(id, state, null, null, null);
    }
}

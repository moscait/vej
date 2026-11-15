package com.moscait.vej;

import com.fasterxml.jackson.annotation.JsonGetter;
import com.fasterxml.jackson.annotation.JsonInclude;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * One question in a multi-question (Jev-format) evaluation.
 *
 * @param type         {@code "choice"}, {@code "noul"} (yes/no probability) or {@code "score"}
 *                     (expected value over numeric levels)
 * @param instructions a String, or any JSON-serializable object, describing the question
 * @param criteria     candidate options: a {@code Map<String,String>} of key to description,
 *                     or a {@code List<String>} of keys; unused for {@code noul}
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public record QuestionSpec(String type, Object instructions, Object criteria) {

    public QuestionSpec {
        if (type == null || type.isBlank()) {
            type = "choice";
        }
    }

    /**
     * Score levels keyed {@code 0..n} are serialized as an ordered list: hosted Jev expects a
     * list for {@code score} criteria, and {@code vej} indexes lists {@code 0..n}.
     */
    @JsonGetter("criteria")
    Object wireCriteria() {
        if (!"score".equalsIgnoreCase(type) || !(criteria instanceof Map<?, ?> map) || map.isEmpty()) {
            return criteria;
        }
        Map<Integer, String> levels = new HashMap<>();
        for (Map.Entry<?, ?> entry : map.entrySet()) {
            if (entry.getKey() == null) {
                return criteria;
            }
            int level;
            try {
                level = Integer.parseInt(entry.getKey().toString().trim());
            } catch (NumberFormatException e) {
                return criteria;
            }
            levels.put(level, entry.getValue() == null ? "" : entry.getValue().toString());
        }
        List<String> ordered = new ArrayList<>(levels.size());
        for (int i = 0; i < levels.size(); i++) {
            String desc = levels.get(i);
            if (desc == null) {
                return criteria;
            }
            ordered.add(desc);
        }
        return ordered;
    }

    /** A categorical question choosing one of {@code criteria}. */
    public static QuestionSpec choice(Object instructions, Object criteria) {
        return new QuestionSpec("choice", instructions, criteria);
    }

    /** A boolean question; the answer carries the probability of {@code true}. */
    public static QuestionSpec noul(Object instructions) {
        return new QuestionSpec("noul", instructions, null);
    }

    /** An expected-value score over numeric levels such as {@code {"0": "...", "5": "..."}}. */
    public static QuestionSpec score(Object instructions, Object criteria) {
        return new QuestionSpec("score", instructions, criteria);
    }
}

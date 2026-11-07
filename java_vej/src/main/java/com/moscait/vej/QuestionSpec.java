package com.moscait.vej;

import com.fasterxml.jackson.annotation.JsonInclude;

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

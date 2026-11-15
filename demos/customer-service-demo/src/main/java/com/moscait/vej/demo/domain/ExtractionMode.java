package com.moscait.vej.demo.domain;

/**
 * Controls what text from a customer service interaction is submitted to {@code vej} and {@code Jev}.
 */
public enum ExtractionMode {
    FIRST_REASON_SENTENCE("First Reason Sentence (Recommended)"),
    FULL_TRANSCRIPT("Full Interaction Transcript");

    private final String label;

    ExtractionMode(String label) {
        this.label = label;
    }

    public String label() {
        return label;
    }
}

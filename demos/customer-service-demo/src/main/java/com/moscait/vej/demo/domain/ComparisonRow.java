package com.moscait.vej.demo.domain;

/**
 * One row in the comparison table representing an example (or custom transcript) and its outcomes.
 */
public final class ComparisonRow {

    public enum RunState {
        PENDING("Pending"),
        RUNNING("Running"),
        DONE("Done");

        private final String label;

        RunState(String label) {
            this.label = label;
        }

        public String label() {
            return label;
        }
    }

    private final CallExample example;
    private String extractedFirstSentence;
    private int extractStartOffset;
    private int extractEndOffset;
    private String evaluatedText;
    private ExtractionMode extractionMode;
    private EngineOutcome vejOutcome;
    private EngineOutcome jevOutcome;
    private RunState runState;

    public ComparisonRow(
            CallExample example,
            String extractedFirstSentence,
            int extractStartOffset,
            int extractEndOffset,
            ExtractionMode extractionMode) {
        this.example = example;
        this.extractedFirstSentence = extractedFirstSentence;
        this.extractStartOffset = extractStartOffset;
        this.extractEndOffset = extractEndOffset;
        this.extractionMode = extractionMode;
        this.evaluatedText = extractionMode == ExtractionMode.FIRST_REASON_SENTENCE
                ? extractedFirstSentence
                : example.transcript();
        this.runState = RunState.PENDING;
    }

    public CallExample example() {
        return example;
    }

    public String extractedFirstSentence() {
        return extractedFirstSentence;
    }

    public int extractStartOffset() {
        return extractStartOffset;
    }

    public int extractEndOffset() {
        return extractEndOffset;
    }

    public String evaluatedText() {
        return evaluatedText;
    }

    public ExtractionMode extractionMode() {
        return extractionMode;
    }

    public void setExtractedPreview(
            String extractedFirstSentence,
            int extractStartOffset,
            int extractEndOffset,
            ExtractionMode mode) {
        this.extractedFirstSentence = extractedFirstSentence;
        this.extractStartOffset = extractStartOffset;
        this.extractEndOffset = extractEndOffset;
        this.extractionMode = mode;
        this.evaluatedText = mode == ExtractionMode.FIRST_REASON_SENTENCE
                ? extractedFirstSentence
                : example.transcript();
    }

    public EngineOutcome vejOutcome() {
        return vejOutcome;
    }

    public void setVejOutcome(EngineOutcome vejOutcome) {
        this.vejOutcome = vejOutcome;
    }

    public EngineOutcome jevOutcome() {
        return jevOutcome;
    }

    public void setJevOutcome(EngineOutcome jevOutcome) {
        this.jevOutcome = jevOutcome;
    }

    public RunState runState() {
        return runState;
    }

    public void setRunState(RunState runState) {
        this.runState = runState;
    }

    public boolean hasBothSuccesses() {
        return vejOutcome != null && vejOutcome.isSuccess()
                && jevOutcome != null && jevOutcome.isSuccess();
    }

    public boolean enginesAgree() {
        return hasBothSuccesses()
                && vejOutcome.rawCategoryChoice().equalsIgnoreCase(jevOutcome.rawCategoryChoice());
    }

    public Boolean isVejCorrect() {
        if (example.expectedCategory() == null || vejOutcome == null || !vejOutcome.isSuccess()) {
            return null;
        }
        return example.expectedCategory() == vejOutcome.predictedCategory();
    }

    public Boolean isJevCorrect() {
        if (example.expectedCategory() == null || jevOutcome == null || !jevOutcome.isSuccess()) {
            return null;
        }
        return example.expectedCategory() == jevOutcome.predictedCategory();
    }

    public double charReductionRatio() {
        int fullLen = example.transcript().length();
        int sentLen = extractedFirstSentence == null ? 0 : extractedFirstSentence.length();
        if (fullLen <= 0 || sentLen >= fullLen) {
            return 0.0;
        }
        return (double) (fullLen - sentLen) / fullLen;
    }
}

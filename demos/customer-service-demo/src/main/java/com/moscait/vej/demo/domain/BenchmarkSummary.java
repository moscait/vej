package com.moscait.vej.demo.domain;

import java.util.List;

/**
 * Aggregate metrics across a set of {@link ComparisonRow}s.
 */
public record BenchmarkSummary(
        int totalRows,
        int completedRows,
        int vejSuccessCount,
        int jevSuccessCount,
        int jevErrorCount,
        String firstJevError,
        int vejCorrectCount,
        int jevCorrectCount,
        int expectedEvaluatedVejCount,
        int expectedEvaluatedJevCount,
        int comparableBothCount,
        int agreementCount,
        double vejAvgClientMs,
        double vejAvgServerMs,
        double jevAvgClientMs,
        double vejAvgConfidence,
        double jevAvgConfidence,
        double vejAvgInputTokens,
        double jevAvgInputTokens,
        double vejAvgOutputTokens,
        double jevAvgOutputTokens,
        double avgCharReductionPct) {

    public static BenchmarkSummary compute(List<ComparisonRow> rows) {
        int total = rows.size();
        int completed = 0;
        int vejSuccess = 0;
        int jevSuccess = 0;
        int jevErrors = 0;
        String firstJevErr = null;
        int vejCorrect = 0;
        int jevCorrect = 0;
        int vejWithExpected = 0;
        int jevWithExpected = 0;
        int bothSuccess = 0;
        int agreements = 0;

        double vejClientSum = 0;
        double vejServerSum = 0;
        int vejServerCount = 0;
        double jevClientSum = 0;
        double vejConfSum = 0;
        double jevConfSum = 0;
        long vejInTokSum = 0;
        long jevInTokSum = 0;
        long vejOutTokSum = 0;
        long jevOutTokSum = 0;
        double charReductionSum = 0;

        for (ComparisonRow row : rows) {
            charReductionSum += row.charReductionRatio();
            if (row.runState() == ComparisonRow.RunState.DONE) {
                completed++;
            }
            EngineOutcome vej = row.vejOutcome();
            if (vej != null && vej.isSuccess()) {
                vejSuccess++;
                vejClientSum += vej.clientLatencyMs();
                if (vej.serverLatencyMs() != null) {
                    vejServerSum += vej.serverLatencyMs();
                    vejServerCount++;
                }
                vejConfSum += vej.categoryConfidence();
                vejInTokSum += vej.inputTokens();
                vejOutTokSum += vej.outputTokens();
                Boolean correct = row.isVejCorrect();
                if (correct != null) {
                    vejWithExpected++;
                    if (correct) vejCorrect++;
                }
            }
            EngineOutcome jev = row.jevOutcome();
            if (jev != null && jev.isSuccess()) {
                jevSuccess++;
                jevClientSum += jev.clientLatencyMs();
                jevConfSum += jev.categoryConfidence();
                jevInTokSum += jev.inputTokens();
                jevOutTokSum += jev.outputTokens();
                Boolean correct = row.isJevCorrect();
                if (correct != null) {
                    jevWithExpected++;
                    if (correct) jevCorrect++;
                }
            } else if (jev != null && jev.errorMessage() != null) {
                jevErrors++;
                if (firstJevErr == null) {
                    firstJevErr = jev.errorMessage();
                }
            }
            if (row.hasBothSuccesses()) {
                bothSuccess++;
                if (row.enginesAgree()) {
                    agreements++;
                }
            }
        }

        return new BenchmarkSummary(
                total,
                completed,
                vejSuccess,
                jevSuccess,
                jevErrors,
                firstJevErr,
                vejCorrect,
                jevCorrect,
                vejWithExpected,
                jevWithExpected,
                bothSuccess,
                agreements,
                vejSuccess == 0 ? 0 : vejClientSum / vejSuccess,
                vejServerCount == 0 ? 0 : vejServerSum / vejServerCount,
                jevSuccess == 0 ? 0 : jevClientSum / jevSuccess,
                vejSuccess == 0 ? 0 : vejConfSum / vejSuccess,
                jevSuccess == 0 ? 0 : jevConfSum / jevSuccess,
                vejSuccess == 0 ? 0 : (double) vejInTokSum / vejSuccess,
                jevSuccess == 0 ? 0 : (double) jevInTokSum / jevSuccess,
                vejSuccess == 0 ? 0 : (double) vejOutTokSum / vejSuccess,
                jevSuccess == 0 ? 0 : (double) jevOutTokSum / jevSuccess,
                total == 0 ? 0 : (charReductionSum / total) * 100.0);
    }

    public double vejAccuracyPct() {
        return expectedEvaluatedVejCount == 0 ? 0 : (vejCorrectCount * 100.0) / expectedEvaluatedVejCount;
    }

    public double jevAccuracyPct() {
        return expectedEvaluatedJevCount == 0 ? 0 : (jevCorrectCount * 100.0) / expectedEvaluatedJevCount;
    }

    public double agreementPct() {
        return comparableBothCount == 0 ? 0 : (agreementCount * 100.0) / comparableBothCount;
    }

    public double speedupFactor() {
        if (vejAvgClientMs <= 0 || jevAvgClientMs <= 0) {
            return 0.0;
        }
        return jevAvgClientMs / vejAvgClientMs;
    }
}

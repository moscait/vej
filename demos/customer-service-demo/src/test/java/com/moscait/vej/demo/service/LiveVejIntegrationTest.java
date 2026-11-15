package com.moscait.vej.demo.service;

import static org.junit.jupiter.api.Assertions.*;

import com.moscait.vej.VejClient;
import com.moscait.vej.demo.domain.CallExample;
import java.time.Duration;
import java.util.Locale;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.condition.EnabledIfSystemProperty;

/**
 * Opt-in integration test exercising all 25 customer service examples against a running local vej daemon.
 * Run with: {@code ./mvnw test -Dvej.live=true}
 */
@EnabledIfSystemProperty(named = "vej.live", matches = "true")
class LiveVejIntegrationTest {

    @Test
    void evaluatesAll25ExamplesAgainstLocalVejDaemon() {
        String baseUrl = System.getProperty("vej.baseUrl", "http://127.0.0.1:8080");
        var status = VejClient.create(baseUrl).status();
        assertTrue(status.isOk(), "Expected live vej daemon to be ok at " + baseUrl);

        var service = new EvaluationComparisonService(
                Duration.ofSeconds(60),
                "jev-latest",
                (url, apiKey, timeout) -> VejClient.builder().baseUrl(url).apiKey(apiKey).timeout(timeout).build());

        int correctFirstSentence = 0;
        int correctFull = 0;
        double sumFirstMs = 0;
        double sumFullMs = 0;
        long sumFirstTok = 0;
        long sumFullTok = 0;

        for (CallExample ex : CustomerServiceExamples.all()) {
            String first = FirstSentenceExtractor.extract(ex.transcript()).firstReasonSentence();

            var firstRes = service.compare(first, baseUrl, null, null, null, null, true, 1.0).vejOutcome();
            assertTrue(firstRes.isSuccess(), "Failed first-sentence eval for " + ex.id() + ": " + firstRes.errorMessage());
            assertNotNull(firstRes.predictedCategory(), "Unmapped category: " + firstRes.rawCategoryChoice());
            assertNotNull(firstRes.urgencyScore());
            assertNotNull(firstRes.escalationNeeded());
            if (firstRes.predictedCategory() == ex.expectedCategory()) {
                correctFirstSentence++;
            }
            sumFirstMs += firstRes.clientLatencyMs();
            sumFirstTok += firstRes.inputTokens();

            var fullRes = service.compare(ex.transcript(), baseUrl, null, null, null, null, true, 1.0).vejOutcome();
            assertTrue(fullRes.isSuccess(), "Failed full-text eval for " + ex.id() + ": " + fullRes.errorMessage());
            if (fullRes.predictedCategory() == ex.expectedCategory()) {
                correctFull++;
            }
            sumFullMs += fullRes.clientLatencyMs();
            sumFullTok += fullRes.inputTokens();

            System.out.printf(
                    Locale.US,
                    "%s | exp=%-20s | 1st=%-20s (%.1f%%, %.0fms, %dt) | full=%-20s (%.1f%%, %.0fms, %dt)%n",
                    ex.id(),
                    ex.expectedCategory().key(),
                    firstRes.rawCategoryChoice(),
                    firstRes.categoryConfidence() * 100,
                    firstRes.clientLatencyMs(),
                    firstRes.inputTokens(),
                    fullRes.rawCategoryChoice(),
                    fullRes.categoryConfidence() * 100,
                    fullRes.clientLatencyMs(),
                    fullRes.inputTokens());
        }

        System.out.printf(
                Locale.US,
                "SUMMARY: 1st-sentence %d/25 (avg %.1f ms, %.1f in tokens) vs full %d/25 (avg %.1f ms, %.1f in tokens)%n",
                correctFirstSentence, sumFirstMs / 25.0, sumFirstTok / 25.0,
                correctFull, sumFullMs / 25.0, sumFullTok / 25.0);

        assertTrue(correctFirstSentence >= 20, "Expected strong accuracy with first-sentence extraction, got " + correctFirstSentence);
    }
}

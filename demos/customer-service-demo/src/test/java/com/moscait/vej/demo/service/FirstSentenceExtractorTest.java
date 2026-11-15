package com.moscait.vej.demo.service;

import static org.junit.jupiter.api.Assertions.*;

import com.moscait.vej.demo.domain.CallExample;
import org.junit.jupiter.api.Test;

class FirstSentenceExtractorTest {

    @Test
    void extractsCustomerReasonAfterAgentAndCallerGreetings() {
        String transcript = """
                Agent: Thank you for calling Northstar Retail, my name is Sam.
                Customer: Hi Sam, hope your morning is going well! I was double-charged $149.99 on my Visa for order #NS-88412 this morning and need the duplicate charge refunded. I only clicked pay once.
                """;

        var result = FirstSentenceExtractor.extract(transcript);
        assertEquals(
                "I was double-charged $149.99 on my Visa for order #NS-88412 this morning and need the duplicate charge refunded.",
                result.firstReasonSentence());
        assertEquals(
                result.firstReasonSentence(),
                transcript.substring(result.startOffset(), result.endOffset()));
    }

    @Test
    void preservesAbbreviationsDecimalsAndTimesInsideSentence() {
        String transcript = """
                Good afternoon, thanks for picking up. My shuttle missed the 6:15 a.m. pickup at St. Jude Ave. outside Terminal B, so I need a $45.50 refund immediately. I have the receipt here.
                """;

        var result = FirstSentenceExtractor.extract(transcript);
        assertEquals(
                "My shuttle missed the 6:15 a.m. pickup at St. Jude Ave. outside Terminal B, so I need a $45.50 refund immediately.",
                result.firstReasonSentence());
    }

    @Test
    void keepsGreetingSentenceWhenItAlreadyContainsReason() {
        String transcript = "Hi, thanks for picking up—I need to cancel my plan right away. Moving out of state tomorrow.";

        var result = FirstSentenceExtractor.extract(transcript);
        assertEquals("Hi, thanks for picking up—I need to cancel my plan right away.", result.firstReasonSentence());
    }

    @Test
    void skipsNameIntroAndMultiWordTeamGreetings() {
        var intro = FirstSentenceExtractor.extract(
                "Hi, my name is Devon. The headphones arrived broken and I want a refund. Thanks.");
        assertEquals("The headphones arrived broken and I want a refund.", intro.firstReasonSentence());

        var team = FirstSentenceExtractor.extract(
                "Hello support team. We need help configuring SAML SSO before Monday. Please advise.");
        assertEquals("We need help configuring SAML SSO before Monday.", team.firstReasonSentence());
    }

    @Test
    void handlesAll25BuiltInExamplesWithExactSubstringOffsets() {
        for (CallExample ex : CustomerServiceExamples.all()) {
            var res = FirstSentenceExtractor.extract(ex.transcript());
            assertFalse(res.firstReasonSentence().isBlank(), "Blank extraction for " + ex.id());
            assertEquals(
                    res.firstReasonSentence(),
                    ex.transcript().substring(res.startOffset(), res.endOffset()),
                    "Offsets must match extracted sentence for " + ex.id());
            assertTrue(
                    res.firstReasonSentence().length() > 30,
                    "Expected substantive reason sentence for " + ex.id() + " but got: " + res.firstReasonSentence());
            assertTrue(
                    res.wasTruncated(),
                    "Expected multi-sentence example " + ex.id() + " to be trimmed compared to full transcript");
        }
    }
}

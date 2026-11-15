package com.moscait.vej.demo.service;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Extracts the first sentence explaining the caller's reason for contacting customer service.
 *
 * <p>Handles both plain multi-sentence utterances and multi-turn {@code Agent: ... Customer: ...}
 * transcripts, skipping pure greeting/small-talk sentences when followed by a substantive reason.
 */
public final class FirstSentenceExtractor {

    private static final Pattern SPEAKER_LINE_PATTERN = Pattern.compile(
            "(?im)^\\s*(agent|representative|rep|support|bot|ivr|advisor|specialist|customer|caller|user|client)\\s*:\\s*");

    private static final Set<String> CUSTOMER_SPEAKERS = Set.of(
            "customer", "caller", "user", "client");

    private static final Set<String> ABBREVIATIONS = Set.of(
            "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "ave", "blvd", "rd", "ln", "ct", "pl",
            "no", "inc", "ltd", "co", "vs", "etc", "e.g", "i.e", "a.m", "p.m", "u.s");

    private static final List<Pattern> GREETING_FILLER_PATTERNS = List.of(
            Pattern.compile("^(?:hi|hello|hey|good morning|good afternoon|good evening)"
                    + "(?:\\s+(?:there|everyone|folks|all|support team|team|"
                    + "(?!(?:my|this|i|i'm|im|thanks|thank|how|hope|sorry|do|it|it's|its|nice|good|great|"
                    + "appreciate|please|yes|yeah|well|we|our|the|a|an|for|to|about|because|calling|call|need|want)\\b)"
                    + "[a-z]{2,18}))?\\b"),
            Pattern.compile("^how are you(?: doing)?(?: today)?\\b"),
            Pattern.compile("^how(?:'s| is) your (?:day|morning|afternoon|week)(?: going)?\\b"),
            Pattern.compile("^(?:i )?hope (?:you(?:'re| are)(?: doing)?|your (?:day|morning|afternoon|week) is(?: going)?) (?:well|good|great)\\b"),
            Pattern.compile("^thank(?:s| you)(?: so much)? for (?:taking my call|picking up|answering|your time|your help today|calling me back)\\b"),
            Pattern.compile("^(?:i )?appreciate you (?:taking my call|picking up|answering|calling me back)\\b"),
            Pattern.compile("^(?:it(?:'s| is) )?(?:nice|good|great) (?:talking|speaking) (?:with|to) you\\b"),
            Pattern.compile("^(?:my name is|this is|i'm|i am) [a-z]{2,18}(?: [a-z]{2,18})?\\b"),
            Pattern.compile("^(?:yes|yeah|yep|sure|okay|ok|um|uh|well)\\b"),
            Pattern.compile("^sorry to (?:bother|trouble) you(?: today)?\\b"),
            Pattern.compile("^do you have a (?:moment|minute|second)\\b")
    );

    private FirstSentenceExtractor() {}

    public record ExtractionResult(
            String firstReasonSentence,
            int startOffset,
            int endOffset,
            int totalLength) {

        public boolean wasTruncated() {
            return !firstReasonSentence.isEmpty() && firstReasonSentence.length() < totalLength;
        }
    }

    public static ExtractionResult extract(String transcript) {
        if (transcript == null || transcript.isBlank()) {
            return new ExtractionResult("", 0, 0, transcript == null ? 0 : transcript.length());
        }

        Span customerRegion = findFirstCustomerRegion(transcript);
        List<Span> sentences = splitSentenceSpans(transcript, customerRegion.start(), customerRegion.end());
        if (sentences.isEmpty()) {
            String fallback = transcript.substring(customerRegion.start(), customerRegion.end()).trim();
            return new ExtractionResult(fallback, customerRegion.start(), customerRegion.end(), transcript.length());
        }

        for (int i = 0; i < sentences.size(); i++) {
            Span span = sentences.get(i);
            String candidate = transcript.substring(span.start(), span.end());
            boolean hasMoreSentences = i < sentences.size() - 1;
            if (hasMoreSentences && isPureGreetingOrIntro(candidate)) {
                continue;
            }
            return new ExtractionResult(candidate, span.start(), span.end(), transcript.length());
        }

        Span first = sentences.get(0);
        return new ExtractionResult(
                transcript.substring(first.start(), first.end()),
                first.start(),
                first.end(),
                transcript.length());
    }

    private record Span(int start, int end) {}

    private static Span findFirstCustomerRegion(String text) {
        Matcher matcher = SPEAKER_LINE_PATTERN.matcher(text);
        List<TurnMarker> markers = new ArrayList<>();
        while (matcher.find()) {
            markers.add(new TurnMarker(
                    matcher.group(1).toLowerCase(Locale.ROOT),
                    matcher.start(),
                    matcher.end()));
        }
        if (markers.isEmpty()) {
            return trimSpan(text, 0, text.length());
        }

        for (int i = 0; i < markers.size(); i++) {
            TurnMarker marker = markers.get(i);
            if (CUSTOMER_SPEAKERS.contains(marker.role())) {
                int end = (i + 1 < markers.size()) ? markers.get(i + 1).lineStart() : text.length();
                return trimSpan(text, marker.contentStart(), end);
            }
        }

        // Transcript has only non-customer labels; fall back to content after first turn or whole text.
        TurnMarker first = markers.get(0);
        int end = markers.size() > 1 ? markers.get(1).lineStart() : text.length();
        return trimSpan(text, first.contentStart(), end);
    }

    private record TurnMarker(String role, int lineStart, int contentStart) {}

    private static Span trimSpan(String text, int start, int end) {
        int s = Math.max(0, start);
        int e = Math.min(text.length(), end);
        while (s < e && Character.isWhitespace(text.charAt(s))) s++;
        while (e > s && Character.isWhitespace(text.charAt(e - 1))) e--;
        return new Span(s, e);
    }

    static List<Span> splitSentenceSpans(String text, int regionStart, int regionEnd) {
        List<Span> spans = new ArrayList<>();
        int currentStart = regionStart;
        int i = regionStart;

        while (i < regionEnd) {
            char c = text.charAt(i);
            if (c == '.' || c == '!' || c == '?') {
                if (c == '.' && isNonTerminalPeriod(text, i, regionStart, regionEnd)) {
                    i++;
                    continue;
                }
                int endPos = i + 1;
                while (endPos < regionEnd && isClosingPunctuation(text.charAt(endPos))) {
                    endPos++;
                }
                if (endPos == regionEnd || Character.isWhitespace(text.charAt(endPos))) {
                    Span trimmed = trimSpan(text, currentStart, endPos);
                    if (trimmed.end() > trimmed.start()) {
                        spans.add(trimmed);
                    }
                    i = endPos;
                    while (i < regionEnd && Character.isWhitespace(text.charAt(i))) {
                        i++;
                    }
                    currentStart = i;
                    continue;
                }
            }
            i++;
        }

        Span tail = trimSpan(text, currentStart, regionEnd);
        if (tail.end() > tail.start()) {
            spans.add(tail);
        }
        return spans;
    }

    private static boolean isClosingPunctuation(char ch) {
        return ch == '"' || ch == '\'' || ch == ')' || ch == ']' || ch == '\u201D' || ch == '\u2019';
    }

    private static boolean isNonTerminalPeriod(String text, int dotIdx, int regionStart, int regionEnd) {
        // Decimal number or version string (e.g., $149.99 or 3.2.1)
        if (dotIdx > regionStart && dotIdx + 1 < regionEnd
                && Character.isDigit(text.charAt(dotIdx - 1))
                && Character.isDigit(text.charAt(dotIdx + 1))) {
            return true;
        }
        // Ellipsis "..." inside a sentence
        if (dotIdx + 1 < regionEnd && text.charAt(dotIdx + 1) == '.') {
            return true;
        }
        if (dotIdx > regionStart && text.charAt(dotIdx - 1) == '.') {
            return true;
        }
        // Common abbreviations like Mr., Dr., Inc., e.g., p.m.
        int wordStart = dotIdx;
        while (wordStart > regionStart && (Character.isLetter(text.charAt(wordStart - 1))
                || text.charAt(wordStart - 1) == '.')) {
            wordStart--;
        }
        String token = text.substring(wordStart, dotIdx).toLowerCase(Locale.ROOT);
        return ABBREVIATIONS.contains(token);
    }

    static boolean isPureGreetingOrIntro(String sentence) {
        if (sentence == null || sentence.isBlank()) {
            return true;
        }
        String remaining = sentence.toLowerCase(Locale.ROOT)
                .replace('\u2019', '\'')
                .replaceAll("[^a-z0-9'\\s]", " ")
                .replaceAll("\\s+", " ")
                .trim();

        boolean matchedAny = true;
        while (matchedAny && !remaining.isEmpty()) {
            matchedAny = false;
            for (Pattern pattern : GREETING_FILLER_PATTERNS) {
                Matcher matcher = pattern.matcher(remaining);
                if (matcher.find()) {
                    remaining = remaining.substring(matcher.end()).trim();
                    matchedAny = true;
                    break;
                }
            }
        }
        if (remaining.isEmpty()) {
            return true;
        }
        // Strip remaining standalone connector words like "and" / "so" if nothing substantive follows
        remaining = remaining.replaceAll("^(?:and|so|just|really|actually)\\s*$", "").trim();
        return remaining.isEmpty();
    }
}

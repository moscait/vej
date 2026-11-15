package com.moscait.vej.demo.ui;

import com.moscait.vej.demo.domain.CallCategory;
import com.moscait.vej.demo.domain.ComparisonRow;
import com.moscait.vej.demo.domain.EngineOutcome;
import com.moscait.vej.demo.domain.ExtractionMode;
import com.vaadin.flow.component.button.Button;
import com.vaadin.flow.component.button.ButtonVariant;
import com.vaadin.flow.component.badge.Badge;
import com.vaadin.flow.component.badge.BadgeVariant;
import com.vaadin.flow.component.card.Card;
import com.vaadin.flow.component.card.CardVariant;
import com.vaadin.flow.component.html.Div;
import com.vaadin.flow.component.html.H4;
import com.vaadin.flow.component.html.Span;
import com.vaadin.flow.component.icon.VaadinIcon;
import com.vaadin.flow.component.orderedlayout.FlexComponent;
import com.vaadin.flow.component.orderedlayout.HorizontalLayout;
import com.vaadin.flow.component.orderedlayout.VerticalLayout;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.function.Consumer;

/**
 * Side-by-side detail breakdown for a selected benchmark row or custom interaction evaluation.
 */
final class DetailInspectionPanel extends Card {

    private final Div body = new Div();
    private Consumer<String> loadIntoTesterHandler;

    DetailInspectionPanel() {
        addThemeVariants(CardVariant.OUTLINED);
        setWidthFull();
        setTitle("Interaction Inspection & Probability Calibration");
        setSubtitle("Select any row above (or run a custom transcript) to inspect the extracted first sentence and full category probabilities.");
        body.getStyle().set("display", "flex").set("flex-direction", "column").set("gap", "var(--vaadin-gap-m)");
        add(body);
    }

    void setOnLoadIntoTester(Consumer<String> handler) {
        this.loadIntoTesterHandler = handler;
    }

    void showRow(ComparisonRow row) {
        body.removeAll();
        if (row == null) {
            body.add(new Span("No interaction selected."));
            return;
        }

        var ex = row.example();
        HorizontalLayout meta = new HorizontalLayout();
        meta.setPadding(false);
        meta.setSpacing(true);
        meta.setAlignItems(FlexComponent.Alignment.CENTER);
        meta.getStyle().set("flex-wrap", "wrap");

        Badge idBadge = new Badge(ex.id());
        idBadge.addThemeVariants(BadgeVariant.CONTRAST, BadgeVariant.SMALL);
        Badge channelBadge = new Badge(ex.channel() + " • " + ex.customerName());
        channelBadge.addThemeVariants(BadgeVariant.SMALL);
        meta.add(idBadge, channelBadge);

        if (ex.expectedCategory() != null) {
            Span expectedLabel = new Span("Expected:");
            expectedLabel.getStyle().set("font-size", "0.85rem").set("color", "var(--vaadin-text-color-secondary)");
            meta.add(expectedLabel, CategoryBadgeFactory.categoryBadge(ex.expectedCategory()));
        }

        Badge modeBadge = new Badge(
                row.extractionMode() == ExtractionMode.FIRST_REASON_SENTENCE
                        ? "Sent: 1st Reason Sentence"
                        : "Sent: Full Transcript");
        modeBadge.addThemeVariants(BadgeVariant.SMALL);
        meta.add(modeBadge);

        if (loadIntoTesterHandler != null) {
            Button loadBtn = new Button("Load into Custom Tester", VaadinIcon.EDIT.create());
            loadBtn.addThemeVariants(ButtonVariant.SMALL, ButtonVariant.TERTIARY);
            loadBtn.addClickListener(e -> loadIntoTesterHandler.accept(ex.transcript()));
            meta.add(loadBtn);
        }

        // Transcript with highlighted extracted sentence
        Div transcriptBlock = buildHighlightedTranscript(row);

        Div columns = new Div();
        columns.addClassName("detail-columns");
        columns.add(
                buildEngineCard("Local vej", row.vejOutcome(), ex.expectedCategory(), false),
                buildEngineCard("Hosted Jev", row.jevOutcome(), ex.expectedCategory(), true));

        body.add(meta, transcriptBlock, columns);
    }

    private Div buildHighlightedTranscript(ComparisonRow row) {
        Div wrapper = new Div();
        wrapper.getStyle().set("display", "flex").set("flex-direction", "column").set("gap", "var(--vaadin-gap-xs)");

        int fullLen = row.example().transcript().length();
        int sentLen = row.extractedFirstSentence().length();
        double savedPct = row.charReductionRatio() * 100.0;

        Span caption = new Span(String.format(
                Locale.US,
                "Full Interaction (%d chars) — Highlighted First Reason Sentence (%d chars, %.0f%% shorter):",
                fullLen, sentLen, savedPct));
        caption.getStyle().set("font-size", "0.85rem").set("color", "var(--vaadin-text-color-secondary)");

        Div transcriptBox = new Div();
        transcriptBox.addClassName("transcript-box");
        String full = row.example().transcript();
        int start = Math.max(0, Math.min(row.extractStartOffset(), full.length()));
        int end = Math.max(start, Math.min(row.extractEndOffset(), full.length()));

        if (end > start) {
            if (start > 0) {
                transcriptBox.add(new Span(full.substring(0, start)));
            }
            Span highlighted = new Span(full.substring(start, end));
            highlighted.addClassName("extracted-highlight");
            transcriptBox.add(highlighted);
            if (end < full.length()) {
                transcriptBox.add(new Span(full.substring(end)));
            }
        } else {
            transcriptBox.add(new Span(full));
        }

        wrapper.add(caption, transcriptBox);
        return wrapper;
    }

    private Card buildEngineCard(String title, EngineOutcome outcome, CallCategory expected, boolean isJev) {
        Card card = new Card();
        card.addThemeVariants(CardVariant.OUTLINED);
        card.setTitle(title);

        if (outcome == null) {
            card.setSubtitle("Not evaluated yet");
            card.add(new Span("Click Run to evaluate this example."));
            return card;
        }
        if (outcome.skipped()) {
            Badge b = new Badge("Skipped");
            b.addThemeVariants(BadgeVariant.CONTRAST, BadgeVariant.SMALL);
            card.setHeaderSuffix(b);
            card.add(new Span(outcome.skipReason()));
            return card;
        }
        if (outcome.errorMessage() != null) {
            Badge b = new Badge("Error");
            b.addThemeVariants(BadgeVariant.ERROR, BadgeVariant.SMALL);
            card.setHeaderSuffix(b);
            Span err = new Span(outcome.errorMessage());
            err.getStyle().set("color", "var(--aura-red-text)");
            card.add(err);
            return card;
        }

        card.setSubtitle("Model: " + (outcome.model() != null ? outcome.model() : "default"));
        card.setHeaderSuffix(CategoryBadgeFactory.outcomeCell(outcome, expected));

        VerticalLayout content = new VerticalLayout();
        content.setPadding(false);
        content.setSpacing(true);

        // Probability bars across all 5 categories
        H4 probHeading = new H4("Category Probability Distribution");
        probHeading.getStyle().set("margin", "0").set("font-size", "0.9rem");
        content.add(probHeading, buildProbabilityBars(outcome, isJev));

        // Secondary questions (urgency & escalation)
        if (outcome.urgencyScore() != null || outcome.escalationProbability() != null) {
            HorizontalLayout secondary = new HorizontalLayout();
            secondary.setPadding(false);
            secondary.setSpacing(true);
            secondary.getStyle().set("flex-wrap", "wrap");
            if (outcome.urgencyScore() != null) {
                Badge urgBadge = new Badge(String.format(Locale.US, "Urgency: %.2f / 5", outcome.urgencyScore()));
                urgBadge.addThemeVariants(
                        BadgeVariant.SMALL,
                        outcome.urgencyScore() >= 3.8 ? BadgeVariant.ERROR
                                : outcome.urgencyScore() >= 2.8 ? BadgeVariant.WARNING : BadgeVariant.SUCCESS);
                secondary.add(urgBadge);
            }
            if (outcome.escalationProbability() != null) {
                boolean esc = Boolean.TRUE.equals(outcome.escalationNeeded());
                Badge escBadge = new Badge(String.format(
                        Locale.US,
                        "Escalation: %s (%.1f%%)",
                        esc ? "Yes" : "No",
                        outcome.escalationProbability() * 100.0));
                escBadge.addThemeVariants(BadgeVariant.SMALL, esc ? BadgeVariant.WARNING : BadgeVariant.CONTRAST);
                secondary.add(escBadge);
            }
            content.add(secondary);
        }

        // Timing & token metrics
        String serverLatencyText = outcome.serverLatencyMs() != null
                ? String.format(Locale.US, "%.1f ms", outcome.serverLatencyMs())
                : "n/a";
        Span timingLine = new Span(String.format(
                Locale.US,
                "Client time: %.1f ms  •  Engine latency: %s  •  Tokens: %d in / %d out",
                outcome.clientLatencyMs(),
                serverLatencyText,
                outcome.inputTokens(),
                outcome.outputTokens()));
        timingLine.addClassName("mono-metric");
        timingLine.getStyle().set("color", "var(--vaadin-text-color-secondary)");
        content.add(timingLine);

        card.add(content);
        return card;
    }

    private Div buildProbabilityBars(EngineOutcome outcome, boolean isJev) {
        Div list = new Div();
        list.addClassName("prob-list");

        Map<String, Double> sorted = new LinkedHashMap<>();
        for (CallCategory c : CallCategory.values()) {
            sorted.put(c.key(), outcome.categoryProbabilities().getOrDefault(c.key(), 0.0));
        }
        outcome.categoryProbabilities().forEach(sorted::putIfAbsent);

        sorted.entrySet().stream()
                .sorted(Map.Entry.<String, Double>comparingByValue(Comparator.reverseOrder()))
                .forEach(entry -> {
                    Div row = new Div();
                    row.addClassName("prob-row");

                    String labelText = CallCategory.fromKey(entry.getKey())
                            .map(CallCategory::displayName)
                            .orElse(entry.getKey());
                    Span name = new Span(labelText);

                    Div track = new Div();
                    track.addClassName("prob-track");
                    Div fill = new Div();
                    fill.addClassName("prob-fill");
                    boolean isWinner = entry.getKey().equalsIgnoreCase(outcome.rawCategoryChoice());
                    if (isWinner) {
                        fill.addClassName(isJev ? "prob-fill-jev" : "prob-fill-winner");
                    }
                    double pct = Math.max(0.0, Math.min(100.0, entry.getValue() * 100.0));
                    fill.getStyle().set("width", String.format(Locale.US, "%.1f%%", pct));
                    track.add(fill);

                    Span pctLabel = new Span(String.format(Locale.US, "%.1f%%", pct));
                    pctLabel.addClassName("mono-metric");
                    pctLabel.getStyle().set("text-align", "right");

                    row.add(name, track, pctLabel);
                    list.add(row);
                });
        return list;
    }
}

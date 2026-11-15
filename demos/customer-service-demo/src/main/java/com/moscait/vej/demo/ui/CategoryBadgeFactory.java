package com.moscait.vej.demo.ui;

import com.moscait.vej.demo.domain.CallCategory;
import com.moscait.vej.demo.domain.EngineOutcome;
import com.vaadin.flow.component.badge.Badge;
import com.vaadin.flow.component.badge.BadgeVariant;
import com.vaadin.flow.component.html.Span;
import com.vaadin.flow.component.icon.VaadinIcon;
import com.vaadin.flow.component.orderedlayout.FlexComponent;
import com.vaadin.flow.component.orderedlayout.HorizontalLayout;
import java.util.Locale;

/**
 * Shared badge and formatting helpers for categories, engine outcomes, and agreements.
 */
final class CategoryBadgeFactory {

    private CategoryBadgeFactory() {}

    static Badge categoryBadge(CallCategory category) {
        if (category == null) {
            Badge b = new Badge("Unknown");
            b.addThemeVariants(BadgeVariant.SMALL);
            return b;
        }
        Badge badge = new Badge(category.displayName());
        badge.addThemeVariants(BadgeVariant.SMALL);
        if (category.badgeVariant() != null) {
            badge.addThemeVariants(category.badgeVariant());
        }
        badge.addClassName(category.auraAccentClass());
        return badge;
    }

    static Badge rawCategoryBadge(String rawChoice, CallCategory category) {
        if (category != null) {
            return categoryBadge(category);
        }
        Badge badge = new Badge(rawChoice == null || rawChoice.isBlank() ? "Unknown" : rawChoice);
        badge.addThemeVariants(BadgeVariant.SMALL, BadgeVariant.CONTRAST);
        return badge;
    }

    static HorizontalLayout outcomeCell(EngineOutcome outcome, CallCategory expected) {
        HorizontalLayout layout = new HorizontalLayout();
        layout.setPadding(false);
        layout.setSpacing(true);
        layout.setAlignItems(FlexComponent.Alignment.CENTER);
        layout.getStyle().set("flex-wrap", "wrap");

        if (outcome == null) {
            Span pending = new Span("—");
            pending.getStyle().set("color", "var(--vaadin-text-color-secondary)");
            layout.add(pending);
            return layout;
        }
        if (outcome.skipped()) {
            Badge skipped = new Badge("Skipped");
            skipped.addThemeVariants(BadgeVariant.SMALL, BadgeVariant.CONTRAST);
            skipped.getElement().setAttribute("title", outcome.skipReason());
            layout.add(skipped);
            return layout;
        }
        if (outcome.errorMessage() != null) {
            Badge err = new Badge("Error", VaadinIcon.EXCLAMATION_CIRCLE.create());
            err.addThemeVariants(BadgeVariant.SMALL, BadgeVariant.ERROR);
            err.getElement().setAttribute("title", outcome.errorMessage());
            Span msg = new Span(truncate(outcome.errorMessage(), 42));
            msg.getStyle().set("font-size", "0.75rem").set("color", "var(--aura-red-text)");
            msg.getElement().setAttribute("title", outcome.errorMessage());
            layout.add(err, msg);
            return layout;
        }

        layout.add(rawCategoryBadge(outcome.rawCategoryChoice(), outcome.predictedCategory()));

        Span conf = new Span(String.format(Locale.US, "%.1f%%", outcome.categoryConfidence() * 100.0));
        conf.addClassName("mono-metric");
        layout.add(conf);

        if (expected != null) {
            boolean correct = expected == outcome.predictedCategory();
            Badge match = new Badge(
                    correct ? "Match" : "Miss",
                    (correct ? VaadinIcon.CHECK : VaadinIcon.CLOSE).create());
            match.addThemeVariants(
                    BadgeVariant.SMALL,
                    BadgeVariant.ICON_ONLY,
                    correct ? BadgeVariant.SUCCESS : BadgeVariant.ERROR);
            match.getElement().setAttribute("title", correct ? "Matches expected category" : "Differs from expected");
            layout.add(match);
        }
        return layout;
    }

    static Badge agreementBadge(boolean hasBoth, boolean agree) {
        if (!hasBoth) {
            Badge dash = new Badge("—");
            dash.addThemeVariants(BadgeVariant.SMALL);
            return dash;
        }
        Badge b = new Badge(agree ? "Agree" : "Differ");
        b.addThemeVariants(BadgeVariant.SMALL, agree ? BadgeVariant.SUCCESS : BadgeVariant.WARNING);
        return b;
    }

    static String truncate(String s, int maxLen) {
        if (s == null) return "";
        String singleLine = s.replaceAll("\\s+", " ").trim();
        return singleLine.length() <= maxLen ? singleLine : singleLine.substring(0, maxLen - 1) + "…";
    }
}

package com.moscait.vej.demo.domain;

import com.vaadin.flow.component.badge.BadgeVariant;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;

/**
 * Target categories for customer service interaction classification.
 */
public enum CallCategory {
    COMPLAINT(
            "complaint",
            "Complaint",
            "Primary purpose is filing a complaint, reporting rude staff or poor service, or expressing dissatisfaction without asking for troubleshooting, a refund, or cancellation",
            BadgeVariant.WARNING,
            "aura-accent-orange"),
    REFUND(
            "refund",
            "Refund",
            "Primary purpose is requesting money back, a refund, reimbursement, reversal of a duplicate charge, or a billing credit",
            BadgeVariant.CONTRAST,
            "aura-accent-purple"),
    SERVICE_CANCELLATION(
            "service_cancellation",
            "Service Cancellation",
            "Primary purpose is canceling a subscription, terminating a plan, closing an account, or stopping recurring billing",
            BadgeVariant.ERROR,
            "aura-accent-red"),
    SUPPORT(
            "support",
            "Support",
            "Primary purpose is technical troubleshooting, fixing a broken feature, resetting access/MFA, setup help, or checking order/delivery instructions",
            null,
            "aura-accent-blue"),
    POSITIVE_FEEDBACK(
            "positive_feedback",
            "Positive Feedback",
            "Primary purpose is thanking an agent, praising a team or product, or sharing positive feedback (even when mentioning a resolved past problem)",
            BadgeVariant.SUCCESS,
            "aura-accent-green");

    private final String key;
    private final String displayName;
    private final String criteriaDescription;
    private final BadgeVariant badgeVariant;
    private final String auraAccentClass;

    CallCategory(
            String key,
            String displayName,
            String criteriaDescription,
            BadgeVariant badgeVariant,
            String auraAccentClass) {
        this.key = key;
        this.displayName = displayName;
        this.criteriaDescription = criteriaDescription;
        this.badgeVariant = badgeVariant;
        this.auraAccentClass = auraAccentClass;
    }

    public String key() {
        return key;
    }

    public String displayName() {
        return displayName;
    }

    public String criteriaDescription() {
        return criteriaDescription;
    }

    public BadgeVariant badgeVariant() {
        return badgeVariant;
    }

    public String auraAccentClass() {
        return auraAccentClass;
    }

    /** Builds the ordered {@code criteria} map shared by {@code vej} and {@code Jev}. */
    public static Map<String, String> toCriteriaMap() {
        Map<String, String> criteria = new LinkedHashMap<>();
        for (CallCategory category : values()) {
            criteria.put(category.key(), category.criteriaDescription());
        }
        return criteria;
    }

    /** Resolves a model response key back to a {@link CallCategory}. */
    public static Optional<CallCategory> fromKey(String rawKey) {
        if (rawKey == null || rawKey.isBlank()) {
            return Optional.empty();
        }
        String normalized = rawKey.trim().toLowerCase(Locale.ROOT).replace('-', '_').replace(' ', '_');
        for (CallCategory category : values()) {
            if (category.key.equals(normalized)) {
                return Optional.of(category);
            }
        }
        return Optional.empty();
    }
}

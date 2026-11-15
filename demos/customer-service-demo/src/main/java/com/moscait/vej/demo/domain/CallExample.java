package com.moscait.vej.demo.domain;

/**
 * One customer service interaction fixture with its expected ground-truth category.
 */
public record CallExample(
        int number,
        String id,
        CallCategory expectedCategory,
        String channel,
        String customerName,
        String transcript) {}

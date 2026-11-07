package com.moscait.vej;

/**
 * Result of {@link VejClient#decideNoul}.
 *
 * @param verdict     {@code true} when {@code probability >= 0.5}
 * @param probability probability of {@code true} (0..1)
 */
public record NoulResult(boolean verdict, double probability) {}

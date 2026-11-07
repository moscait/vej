package com.moscait.vej;

import java.util.List;

/** One entry in the daemon's model catalog ({@code GET /v1/models}). */
public record ModelInfo(
        String id,
        String name,
        String vendor,
        String country,
        double sizeGb,
        List<String> aliases,
        boolean isDownloaded,
        boolean isActive) {

    public ModelInfo {
        aliases = aliases == null ? List.of() : List.copyOf(aliases);
    }
}

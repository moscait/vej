package com.moscait.vej;

/** Daemon health ({@code GET /health}). */
public record DaemonStatus(String status, String activeModel, String version) {

    /** {@code true} when the daemon reports {@code "ok"}. */
    public boolean isOk() {
        return "ok".equals(status);
    }
}

package com.moscait.vej;

/** Thrown when the daemon (or hosted endpoint) cannot be reached or does not respond in time. */
public class VejConnectionException extends VejException {
    private static final long serialVersionUID = 1L;

    public VejConnectionException(String message, Throwable cause) {
        super(message, null, null, cause);
    }
}

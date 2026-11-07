package com.moscait.vej;

/**
 * Thrown when a vej API request fails. Unchecked, so callers opt in to handling.
 *
 * <p>Subclasses distinguish connection failures ({@link VejConnectionException}) and
 * client-side validation failures ({@link VejValidationException}).
 */
public class VejException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    private final Integer statusCode;
    private final String responseBody;

    public VejException(String message) {
        this(message, null, null, null);
    }

    public VejException(String message, Integer statusCode, String responseBody) {
        this(message, statusCode, responseBody, null);
    }

    public VejException(String message, Integer statusCode, String responseBody, Throwable cause) {
        super(statusCode != null ? "HTTP " + statusCode + ": " + message : message, cause);
        this.statusCode = statusCode;
        this.responseBody = responseBody;
    }

    /** HTTP status code if the error came from an HTTP response, else {@code null}. */
    public Integer statusCode() {
        return statusCode;
    }

    /** Raw response body if the error came from an HTTP response, else {@code null}. */
    public String responseBody() {
        return responseBody;
    }
}

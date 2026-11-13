package com.moscait.vej;

/** Thrown when request arguments fail client-side validation, before any network call. */
public class VejValidationException extends VejException {
    private static final long serialVersionUID = 1L;

    public VejValidationException(String message) {
        super(message);
    }
}

/// Exception thrown when a vej API or engine request fails.
class VejException implements Exception {
  /// Error message.
  final String message;

  /// Optional HTTP status code if the error originated from an HTTP response.
  final int? statusCode;

  /// Optional underlying response body or detail.
  final dynamic details;

  const VejException(this.message, {this.statusCode, this.details});

  @override
  String toString() {
    if (statusCode != null) {
      return 'VejException (HTTP $statusCode): $message';
    }
    return 'VejException: $message';
  }
}

/// Thrown when a network connection to the daemon or endpoint cannot be established.
class VejConnectionException extends VejException {
  const VejConnectionException(super.message, {super.details});

  @override
  String toString() => 'VejConnectionException: $message';
}

/// Thrown when request arguments or options fail client-side validation.
class VejValidationException extends VejException {
  const VejValidationException(super.message);

  @override
  String toString() => 'VejValidationException: $message';
}

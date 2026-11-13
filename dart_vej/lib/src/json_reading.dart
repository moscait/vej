/// Typed reads of decoded JSON. A value of the wrong shape throws [FormatException], which
/// the client reports as an invalid response instead of letting a `TypeError` escape.
library;

/// [value] as a JSON object, or [FormatException] naming [what].
Map<String, dynamic> asObject(Object? value, String what) => switch (value) {
      Map<String, dynamic> map => map,
      _ => throw FormatException('expected $what to be a JSON object'),
    };

/// [value] as a JSON array, or [FormatException] naming [what].
List<dynamic> asList(Object? value, String what) => switch (value) {
      List<dynamic> list => list,
      _ => throw FormatException('expected $what to be a JSON array'),
    };

/// The object at [key], an empty map when absent or null.
Map<String, dynamic> optObject(Map<String, dynamic> json, String key) =>
    json[key] == null ? const {} : asObject(json[key], "'$key'");

/// The array at [key], an empty list when absent or null.
List<dynamic> optList(Map<String, dynamic> json, String key) =>
    json[key] == null ? const [] : asList(json[key], "'$key'");

/// The number at [key] as a double, null when absent.
double? optDouble(Map<String, dynamic> json, String key) => switch (json[key]) {
      null => null,
      num n => n.toDouble(),
      _ => throw FormatException("expected '$key' to be a number"),
    };

/// The number at [key] as an int, null when absent.
int? optInt(Map<String, dynamic> json, String key) => switch (json[key]) {
      null => null,
      num n => n.toInt(),
      _ => throw FormatException("expected '$key' to be a number"),
    };

/// The bool at [key], null when absent.
bool? optBool(Map<String, dynamic> json, String key) => switch (json[key]) {
      null => null,
      bool b => b,
      _ => throw FormatException("expected '$key' to be true or false"),
    };

/// The string at [key], null when absent.
String? optString(Map<String, dynamic> json, String key) => switch (json[key]) {
      null => null,
      String s => s,
      _ => throw FormatException("expected '$key' to be a string"),
    };

/// The string at [key], or [FormatException] when absent.
String reqString(Map<String, dynamic> json, String key) =>
    optString(json, key) ?? (throw FormatException("missing '$key'"));

/// The number at [key] as a double, or [FormatException] when absent.
double reqDouble(Map<String, dynamic> json, String key) =>
    optDouble(json, key) ?? (throw FormatException("missing '$key'"));

/// The object at [key] read as a map of numbers, e.g. a probability distribution.
Map<String, double> doubleMap(Map<String, dynamic> json, String key) =>
    optObject(json, key).map(
      (k, v) => MapEntry(
        k,
        switch (v) {
          num n => n.toDouble(),
          _ => throw FormatException("expected '$key.$k' to be a number"),
        },
      ),
    );

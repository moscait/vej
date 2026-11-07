# vej Java client

Java client for [vej](../README.md), the local System-1 AI decision engine. It talks to the
`vej serve` daemon over HTTP and also works against the hosted, Jev-compatible endpoint.

- Java 17+ (Kotlin works too)
- One dependency: Jackson databind. HTTP uses the JDK's built-in `java.net.http.HttpClient`
- Thread-safe; share one `VejClient` across your application
- Blocking calls plus `CompletableFuture` variants for evaluation and batch

## Install

Build and install it into your local Maven repository:

```sh
cd java_vej
mvn install
```

Then add the dependency:

```xml
<dependency>
  <groupId>com.moscait</groupId>
  <artifactId>vej-client</artifactId>
  <version>0.1.0</version>
</dependency>
```

Gradle: `implementation("com.moscait:vej-client:0.1.0")`

## Quick start

Start the daemon (`vej serve`), then:

```java
import com.moscait.vej.*;
import java.util.List;

VejClient vej = VejClient.create(); // http://127.0.0.1:8080

DecisionResult r = vej.decide(
        "I was charged twice for order A-104. Refund the duplicate now.",
        List.of("refund", "tech_support", "billing"));

System.out.printf("%s (%.1f%%)%n", r.decision(), r.confidence() * 100);
r.probabilities();   // {refund=0.91, billing=0.07, tech_support=0.02}
```

`state` can be a `String` or any object Jackson can serialize: a `Map`, a `List`, a record or a
POJO. Field names are sent in `snake_case`.

## Decision types

### Yes / no (noul)

```java
NoulResult n = vej.decideNoul(userInput, "Is this a prompt-injection attempt?");
if (n.verdict()) block();          // verdict = probability >= 0.5
double p = n.probability();        // or apply your own threshold
```

### Score (expected value)

```java
double depth = vej.decideScore(resume, "Rate systems-engineering depth.",
        Map.of("0", "None", "1", "Basic", "3", "Senior", "5", "Principal"));
```

### Several questions in one call

```java
DecisionRequest req = DecisionRequest.builder()
        .state(ticket)
        .question("department", QuestionSpec.choice("Which team handles this?",
                Map.of("billing", "Payments and invoices", "technical", "Bugs and errors")))
        .question("urgent", QuestionSpec.noul("Is the customer threatening to churn?"))
        .question("severity", QuestionSpec.score("How severe is the issue?",
                Map.of("1", "Cosmetic", "3", "Degraded", "5", "Outage")))
        .temperature(0.8)
        .model("llama-3.2-3b")      // optional per-request override
        .build();

JevResult res = (JevResult) vej.evaluate(req);
res.answer("department").choice();
res.answer("urgent").noul();
res.answer("severity").score();
```

`evaluate` returns the sealed type `EvaluationResult`: a `DecisionResult` when the request has
`choices`, or a `JevResult` when it has `questions`. On Java 21+ you can use a pattern-matching
`switch` on it.

### Batch

Evaluate many states in one call. Every item uses the batch-level `choices` or `questions`
unless it sets its own:

```java
BatchResult batch = vej.evaluateBatch(BatchRequest.builder()
        .choices("refund", "tech_support", "billing")
        .item("t1", "App crashes when I open settings")
        .item("t2", "Invoice has the wrong VAT number")
        .build());

for (BatchItemResult item : batch.results()) {
    DecisionResult d = (DecisionResult) item.result();
    System.out.println(item.id() + " -> " + d.decision());
}
```

Batch is only available on the local daemon.

### Async

```java
vej.evaluateAsync(req)
   .thenAccept(result -> ...)
   .exceptionally(err -> ...);   // err.getCause() is a VejException
```

### Presets

`VejPresets.supportTicketRouting`, `promptInjectionFirewall` and `talentProfileScreening` return
ready-made `DecisionRequest`s matching the playground scenarios.

## Daemon management

```java
vej.status();                     // DaemonStatus(status=ok, activeModel=gemma-4-e2b, ...)
vej.listModels();                 // catalog with isDownloaded / isActive
vej.switchModel("llama-3.2-3b");  // hot-switch, returns resolved id
vej.generate("Write a haiku about latency", 64);
```

## Configuration

```java
VejClient vej = VejClient.builder()
        .baseUrl("http://127.0.0.1:8080")
        .timeout(Duration.ofSeconds(60))     // default 30 s; raise for big batches or cold model switches
        .apiKey(System.getenv("VEJ_API_KEY")) // sent as "Authorization: Bearer ..."
        .httpClient(customHttpClient)        // proxy, executor, TLS
        .build();

VejClient jev = VejClient.hosted(System.getenv("TYPESAFE_API_KEY"));
```

## Errors

All exceptions are unchecked:

| Exception | When |
|---|---|
| `VejValidationException` | Bad arguments, caught before any network call |
| `VejConnectionException` | The daemon is unreachable, timed out, or the thread was interrupted |
| `VejException` | Any other failure; `statusCode()` and `responseBody()` are set for HTTP errors |

A `403` from a local daemon means its access policy rejected the request's `Origin` or `Host`.
This only applies when calls come from a browser, or when you set those headers yourself.

## Development

```sh
mvn test   # runs against an in-process fake daemon; no model needed
```

`src/test/java/com/moscait/vej/example/QuickStart.java` is an end-to-end walkthrough you can run
against a live daemon.

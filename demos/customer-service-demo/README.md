# Customer Service Call Categorization Demo (`vej` vs `Jev`)

A Java 21+ / Spring Boot 4 + Vaadin 25.3 Flow web application demonstrating local System-1 AI evaluation with [`vej`](../../README.md) vs. hosted TypeSafe AI **Jev**.

## What It Demonstrates

1. **First Reason-for-Call Sentence Extraction (`FirstSentenceExtractor`)**
   - Multi-turn customer service transcripts (`Agent: ... Customer: ...`, greetings, pleasantries, follow-up backstory) are parsed to extract **only the first substantive sentence explaining the reason for the call**.
   - Toggle between **First Reason Sentence** and **Full Interaction Transcript** to observe character/token reduction and latency savings.
   - Click any row to inspect the full transcript with the extracted sentence highlighted in place.

2. **25 Curated Customer Service Examples (5 per Category)**
   - `complaint`
   - `refund`
   - `service_cancellation`
   - `support`
   - `positive_feedback`

3. **Multi-Primitive System-1 Evaluation (`questions` Schema)**
   - **`category`** (`choice`): Classifies the interaction across the 5 categories using shared semantic `criteria`.
   - **`urgency`** (`score`, optional toggle): Expected-value urgency score (`0.0` to `5.0`).
   - **`escalation_needed`** (`noul`, optional toggle): Calibrated yes/no probability for supervisor escalation.

4. **Side-by-Side Benchmarking (`vej` vs `Jev`)**
   - Streams live row-by-row updates via Vaadin `@Push`.
   - Displays **client wall-clock turnaround (ms)**, **server engine latency (ms)**, **input/output tokens**, **calibrated softmax probability distributions**, **accuracy vs. expected labels**, and **`vej` ↔ `Jev` agreement**.
   - Works in **`vej`-only** mode immediately; enter a TypeSafe AI API token (or set `TYPESAFE_API_KEY`) to enable **Dual Compare** against `https://api.typesafe.ai`.

5. **Managed Local `vej` Daemon**
   - Connects automatically to an existing `vej serve` daemon at `http://127.0.0.1:8080` (configurable in the UI), **or** auto-starts a local `vej` instance from `../../target/release/vej` (or `../../target/debug/vej` / `PATH`) using an already-downloaded local model (`llama-3.2-3b`, `llama-3.2-1b`, etc.).
   - Vaadin runs on port **`8081`** (`server.port=8081`) to avoid colliding with `vej` on **`8080`**.

---

## Prerequisites

1. **Java 21+** (tested on Java 25) and **Maven 3.8+** (or `./mvnw`).
2. **Install `vej-client` into your local Maven repository** (one-time, or after changes to `java_vej`):
   ```bash
   cd ../../java_vej
   mvn install
   ```
3. **Ensure at least one local `vej` model is downloaded** (if not using an already-running `vej serve`):
   ```bash
   ../../target/release/vej models list
   # e.g. if none are downloaded yet:
   ../../target/release/vej models pull llama-3.2-3b
   ```

---

## Run the Demo

```bash
cd demos/customer-service-demo
./mvnw spring-boot:run
```

Then open:

👉 **[http://localhost:8081](http://localhost:8081)**

Optional environment variables:

```bash
export TYPESAFE_API_KEY="your-jev-api-token"   # Pre-fills the Jev API token field
export VEJ_BASE_URL="http://127.0.0.1:8080"    # Local vej endpoint
export VEJ_AUTO_START="true"                   # Auto-start local vej if offline
export VEJ_MODEL="llama-3.2-3b"                # Override preferred model when auto-starting
```

## Run Tests

```bash
./mvnw test
```

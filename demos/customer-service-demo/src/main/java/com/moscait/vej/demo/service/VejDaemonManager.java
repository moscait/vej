package com.moscait.vej.demo.service;

import com.moscait.vej.DaemonStatus;
import com.moscait.vej.ModelInfo;
import com.moscait.vej.VejClient;
import com.moscait.vej.VejException;
import jakarta.annotation.PreDestroy;
import java.io.BufferedReader;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;
import java.util.Locale;
import java.util.Optional;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.boot.context.event.ApplicationReadyEvent;
import org.springframework.context.event.EventListener;
import org.springframework.stereotype.Service;

/**
 * Monitors the local {@code vej} daemon and optionally starts/manages a local {@code vej serve}
 * child process when none is already listening.
 */
@Service
public class VejDaemonManager {

    private static final Logger log = LoggerFactory.getLogger(VejDaemonManager.class);

    private static final List<String> PREFERRED_MODEL_ORDER = List.of(
            "llama-3.2-3b",
            "llama-3.2-1b",
            "gemma-4-e2b",
            "granite-4.2-3b",
            "smollm2-1.7b",
            "llama-3.1-8b");

    private static final int MAX_LOG_LINES = 150;

    public enum DaemonPhase {
        STOPPED,
        STARTING,
        CONNECTED,
        ERROR
    }

    public record DaemonSnapshot(
            DaemonPhase phase,
            String baseUrl,
            String activeModel,
            String version,
            boolean managedByApp,
            String binaryPath,
            List<ModelInfo> models,
            String statusMessage) {}

    private final String configuredBaseUrl;
    private final boolean autoStart;
    private final String configuredBinaryPath;
    private final String configuredDefaultModel;
    private final ExecutorService lifecycleExecutor = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "vej-daemon-lifecycle");
        t.setDaemon(true);
        return t;
    });
    private final CopyOnWriteArrayList<Consumer<DaemonSnapshot>> listeners = new CopyOnWriteArrayList<>();
    private final Deque<String> recentLogs = new ArrayDeque<>(MAX_LOG_LINES);

    private volatile Process managedProcess;
    private volatile DaemonSnapshot currentSnapshot;

    public VejDaemonManager(
            @Value("${vej.base-url:http://127.0.0.1:8080}") String configuredBaseUrl,
            @Value("${vej.auto-start:true}") boolean autoStart,
            @Value("${vej.binary-path:}") String configuredBinaryPath,
            @Value("${vej.default-model:}") String configuredDefaultModel) {
        this.configuredBaseUrl = normalizeBaseUrl(configuredBaseUrl);
        this.autoStart = autoStart;
        this.configuredBinaryPath = configuredBinaryPath == null ? "" : configuredBinaryPath.trim();
        this.configuredDefaultModel = configuredDefaultModel == null ? "" : configuredDefaultModel.trim();
        this.currentSnapshot = new DaemonSnapshot(
                DaemonPhase.STOPPED,
                this.configuredBaseUrl,
                "",
                "",
                false,
                findVejBinary().map(Path::toString).orElse(""),
                List.of(),
                "Checking local vej daemon...");
    }

    @EventListener(ApplicationReadyEvent.class)
    public void onApplicationReady() {
        lifecycleExecutor.submit(() -> {
            DaemonSnapshot snap = refreshStatus(configuredBaseUrl);
            if (snap.phase() != DaemonPhase.CONNECTED && autoStart && isLoopbackUrl(configuredBaseUrl)) {
                startManagedDaemon(configuredBaseUrl, configuredDefaultModel);
            }
        });
    }

    public DaemonSnapshot getSnapshot() {
        return currentSnapshot;
    }

    public void addListener(Consumer<DaemonSnapshot> listener) {
        listeners.add(listener);
    }

    public void removeListener(Consumer<DaemonSnapshot> listener) {
        listeners.remove(listener);
    }

    public synchronized List<String> getRecentLogs() {
        return List.copyOf(recentLogs);
    }

    public DaemonSnapshot refreshStatus(String baseUrl) {
        String normalized = normalizeBaseUrl(baseUrl);
        String binaryPath = findVejBinary().map(Path::toString).orElse("");
        VejClient client = VejClient.builder()
                .baseUrl(normalized)
                .timeout(Duration.ofSeconds(5))
                .build();
        try {
            DaemonStatus status = client.status();
            List<ModelInfo> models = List.of();
            try {
                models = client.listModels();
            } catch (VejException ignored) {
                // Keep empty catalog if /v1/models is unavailable.
            }
            boolean managed = managedProcess != null && managedProcess.isAlive();
            DaemonSnapshot snap = new DaemonSnapshot(
                    status.isOk() ? DaemonPhase.CONNECTED : DaemonPhase.ERROR,
                    normalized,
                    nonNull(status.activeModel()),
                    nonNull(status.version()),
                    managed,
                    binaryPath,
                    models,
                    status.isOk()
                            ? (managed ? "Running (managed by demo)" : "Connected to external vej daemon")
                            : "Daemon status: " + status.status());
            publish(snap);
            return snap;
        } catch (VejException e) {
            boolean managedAlive = managedProcess != null && managedProcess.isAlive();
            DaemonSnapshot snap = new DaemonSnapshot(
                    managedAlive ? DaemonPhase.STARTING : DaemonPhase.STOPPED,
                    normalized,
                    "",
                    "",
                    managedAlive,
                    binaryPath,
                    List.of(),
                    managedAlive ? "Starting local vej daemon..." : "Offline: " + e.getMessage());
            publish(snap);
            return snap;
        }
    }

    public void startManagedDaemonAsync(String baseUrl, String preferredModel) {
        lifecycleExecutor.submit(() -> startManagedDaemon(baseUrl, preferredModel));
    }

    public synchronized DaemonSnapshot startManagedDaemon(String baseUrl, String preferredModel) {
        String normalized = normalizeBaseUrl(baseUrl);
        DaemonSnapshot existing = refreshStatus(normalized);
        if (existing.phase() == DaemonPhase.CONNECTED) {
            return existing;
        }
        if (!isLoopbackUrl(normalized)) {
            DaemonSnapshot err = new DaemonSnapshot(
                    DaemonPhase.ERROR,
                    normalized,
                    "",
                    "",
                    false,
                    existing.binaryPath(),
                    List.of(),
                    "Auto-start is only supported for localhost / 127.0.0.1 URLs.");
            publish(err);
            return err;
        }

        Optional<Path> binaryOpt = findVejBinary();
        if (binaryOpt.isEmpty()) {
            DaemonSnapshot err = new DaemonSnapshot(
                    DaemonPhase.ERROR,
                    normalized,
                    "",
                    "",
                    false,
                    "",
                    List.of(),
                    "Could not find 'vej' binary in ../../target/release/vej, ../../target/debug/vej, or PATH.");
            publish(err);
            return err;
        }
        Path binary = binaryOpt.get();

        if (managedProcess != null && managedProcess.isAlive()) {
            return refreshStatus(normalized);
        }

        String modelToLoad = resolveModelToStart(binary, preferredModel);
        URI uri = URI.create(normalized);
        String host = uri.getHost() != null ? uri.getHost() : "127.0.0.1";
        int port = uri.getPort() > 0 ? uri.getPort() : 8080;

        List<String> cmd = new ArrayList<>();
        cmd.add(binary.toString());
        cmd.add("serve");
        cmd.add("--host");
        cmd.add(host);
        cmd.add("--port");
        cmd.add(String.valueOf(port));
        if (modelToLoad != null && !modelToLoad.isBlank()) {
            cmd.add("-m");
            cmd.add(modelToLoad);
        }

        String startingMsg = "Starting " + binary.getFileName() + " on " + host + ":" + port
                + (modelToLoad.isBlank() ? "" : " (" + modelToLoad + ")") + "...";
        publish(new DaemonSnapshot(
                DaemonPhase.STARTING,
                normalized,
                modelToLoad,
                "",
                true,
                binary.toString(),
                List.of(),
                startingMsg));
        appendLog("[demo] " + String.join(" ", cmd));

        try {
            ProcessBuilder pb = new ProcessBuilder(cmd);
            pb.redirectErrorStream(true);
            Process process = pb.start();
            managedProcess = process;
            startLogReader(process);

            long deadlineNs = System.nanoTime() + TimeUnit.SECONDS.toNanos(40);
            while (System.nanoTime() < deadlineNs) {
                if (!process.isAlive()) {
                    String errMsg = "vej exited with code " + process.exitValue() + ". Check logs.";
                    DaemonSnapshot err = new DaemonSnapshot(
                            DaemonPhase.ERROR,
                            normalized,
                            "",
                            "",
                            false,
                            binary.toString(),
                            List.of(),
                            errMsg);
                    publish(err);
                    return err;
                }
                DaemonSnapshot check = refreshStatus(normalized);
                if (check.phase() == DaemonPhase.CONNECTED) {
                    return check;
                }
                try {
                    Thread.sleep(250);
                } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt();
                    break;
                }
            }
            DaemonSnapshot timeout = new DaemonSnapshot(
                    DaemonPhase.ERROR,
                    normalized,
                    modelToLoad,
                    "",
                    process.isAlive(),
                    binary.toString(),
                    List.of(),
                    "Timed out waiting for vej /health on " + normalized);
            publish(timeout);
            return timeout;
        } catch (IOException e) {
            log.warn("Failed to start vej daemon", e);
            DaemonSnapshot err = new DaemonSnapshot(
                    DaemonPhase.ERROR,
                    normalized,
                    "",
                    "",
                    false,
                    binary.toString(),
                    List.of(),
                    "Failed to launch vej: " + e.getMessage());
            publish(err);
            return err;
        }
    }

    @PreDestroy
    public synchronized void stopManagedDaemon() {
        Process process = managedProcess;
        managedProcess = null;
        if (process != null && process.isAlive()) {
            appendLog("[demo] Stopping managed vej process...");
            process.destroy();
            try {
                if (!process.waitFor(3, TimeUnit.SECONDS)) {
                    process.destroyForcibly();
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                process.destroyForcibly();
            }
        }
        refreshStatus(currentSnapshot.baseUrl());
    }

    public boolean hasManagedProcess() {
        return managedProcess != null && managedProcess.isAlive();
    }

    Optional<Path> findVejBinary() {
        if (!configuredBinaryPath.isBlank()) {
            Path p = Path.of(configuredBinaryPath).toAbsolutePath().normalize();
            if (Files.isExecutable(p)) {
                return Optional.of(p);
            }
        }
        List<Path> candidates = List.of(
                Path.of("../../target/release/vej"),
                Path.of("target/release/vej"),
                Path.of("../../target/debug/vej"),
                Path.of("target/debug/vej"));
        for (Path candidate : candidates) {
            Path normalized = candidate.toAbsolutePath().normalize();
            if (Files.isExecutable(normalized) && Files.isRegularFile(normalized)) {
                return Optional.of(normalized);
            }
        }
        String pathEnv = System.getenv("PATH");
        if (pathEnv != null) {
            for (String dir : pathEnv.split(File.pathSeparator)) {
                if (dir.isBlank()) continue;
                Path candidate = Path.of(dir, "vej");
                if (Files.isExecutable(candidate) && Files.isRegularFile(candidate)) {
                    return Optional.of(candidate.toAbsolutePath().normalize());
                }
            }
        }
        return Optional.empty();
    }

    private String resolveModelToStart(Path binary, String requestedModel) {
        if (requestedModel != null && !requestedModel.isBlank()) {
            return requestedModel.trim();
        }
        if (!configuredDefaultModel.isBlank()) {
            return configuredDefaultModel;
        }
        List<String> downloaded = queryDownloadedModels(binary);
        for (String preferred : PREFERRED_MODEL_ORDER) {
            if (downloaded.contains(preferred)) {
                return preferred;
            }
        }
        if (!downloaded.isEmpty()) {
            return downloaded.get(0);
        }
        return "llama-3.2-3b";
    }

    private List<String> queryDownloadedModels(Path binary) {
        List<String> downloaded = new ArrayList<>();
        try {
            Process p = new ProcessBuilder(binary.toString(), "models", "list")
                    .redirectErrorStream(true)
                    .start();
            try (BufferedReader reader = new BufferedReader(
                    new InputStreamReader(p.getInputStream(), StandardCharsets.UTF_8))) {
                String line;
                while ((line = reader.readLine()) != null) {
                    if (line.contains("Downloaded")) {
                        String[] tokens = line.trim().split("\\s+");
                        if (tokens.length > 0 && !tokens[0].isBlank()) {
                            downloaded.add(tokens[0].trim());
                        }
                    }
                }
            }
            p.waitFor(5, TimeUnit.SECONDS);
        } catch (Exception e) {
            log.debug("Could not inspect cached vej models: {}", e.getMessage());
        }
        return downloaded;
    }

    private void startLogReader(Process process) {
        Thread readerThread = new Thread(() -> {
            try (BufferedReader reader = new BufferedReader(
                    new InputStreamReader(process.getInputStream(), StandardCharsets.UTF_8))) {
                String line;
                while ((line = reader.readLine()) != null) {
                    appendLog(line);
                }
            } catch (IOException ignored) {
                // Stream closed on process exit.
            }
        }, "vej-daemon-logs");
        readerThread.setDaemon(true);
        readerThread.start();
    }

    private synchronized void appendLog(String line) {
        if (recentLogs.size() >= MAX_LOG_LINES) {
            recentLogs.removeFirst();
        }
        recentLogs.addLast(line);
    }

    private void publish(DaemonSnapshot snapshot) {
        this.currentSnapshot = snapshot;
        for (Consumer<DaemonSnapshot> listener : listeners) {
            try {
                listener.accept(snapshot);
            } catch (Exception e) {
                log.debug("Daemon listener error: {}", e.getMessage());
            }
        }
    }

    private static String normalizeBaseUrl(String raw) {
        String url = (raw == null || raw.isBlank()) ? VejClient.DEFAULT_BASE_URL : raw.trim();
        while (url.endsWith("/")) {
            url = url.substring(0, url.length() - 1);
        }
        return url;
    }

    private static boolean isLoopbackUrl(String baseUrl) {
        try {
            String host = URI.create(baseUrl).getHost();
            if (host == null) return false;
            String lower = host.toLowerCase(Locale.ROOT);
            return "127.0.0.1".equals(lower) || "localhost".equals(lower) || "::1".equals(lower);
        } catch (Exception e) {
            return false;
        }
    }

    private static String nonNull(String s) {
        return s == null ? "" : s;
    }
}

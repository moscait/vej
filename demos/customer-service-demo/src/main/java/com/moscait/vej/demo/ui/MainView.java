package com.moscait.vej.demo.ui;

import com.moscait.vej.ModelInfo;
import com.moscait.vej.VejClient;
import com.moscait.vej.VejException;
import com.moscait.vej.demo.domain.BenchmarkSummary;
import com.moscait.vej.demo.domain.CallCategory;
import com.moscait.vej.demo.domain.CallExample;
import com.moscait.vej.demo.domain.ComparisonRow;
import com.moscait.vej.demo.domain.EngineOutcome;
import com.moscait.vej.demo.domain.ExtractionMode;
import com.moscait.vej.demo.service.CustomerServiceExamples;
import com.moscait.vej.demo.service.EvaluationComparisonService;
import com.moscait.vej.demo.service.FirstSentenceExtractor;
import com.moscait.vej.demo.service.VejDaemonManager;
import com.vaadin.flow.component.AttachEvent;
import com.vaadin.flow.component.DetachEvent;
import com.vaadin.flow.component.UI;
import com.vaadin.flow.component.badge.Badge;
import com.vaadin.flow.component.badge.BadgeVariant;
import com.vaadin.flow.component.button.Button;
import com.vaadin.flow.component.button.ButtonVariant;
import com.vaadin.flow.component.card.Card;
import com.vaadin.flow.component.card.CardVariant;
import com.vaadin.flow.component.checkbox.Switch;
import com.vaadin.flow.component.details.Details;
import com.vaadin.flow.component.dialog.Dialog;
import com.vaadin.flow.component.grid.Grid;
import com.vaadin.flow.component.grid.GridVariant;
import com.vaadin.flow.component.html.Div;
import com.vaadin.flow.component.html.H2;
import com.vaadin.flow.component.html.Span;
import com.vaadin.flow.component.icon.VaadinIcon;
import com.vaadin.flow.component.notification.Notification;
import com.vaadin.flow.component.notification.NotificationVariant;
import com.vaadin.flow.component.orderedlayout.FlexComponent;
import com.vaadin.flow.component.orderedlayout.HorizontalLayout;
import com.vaadin.flow.component.orderedlayout.VerticalLayout;
import com.vaadin.flow.component.progressbar.ProgressBar;
import com.vaadin.flow.component.select.Select;
import com.vaadin.flow.component.textfield.PasswordField;
import com.vaadin.flow.component.textfield.TextArea;
import com.vaadin.flow.component.textfield.TextField;
import com.vaadin.flow.data.provider.ListDataProvider;
import com.vaadin.flow.data.value.ValueChangeMode;
import com.vaadin.flow.router.PageTitle;
import com.vaadin.flow.router.Route;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;
import org.springframework.beans.factory.annotation.Value;

/**
 * Interactive Vaadin UI comparing local {@code vej} against hosted {@code Jev} on 25+
 * customer service call interactions using first-sentence reason extraction.
 */
@Route("")
@PageTitle("vej vs Jev — Customer Service Call Triage")
public class MainView extends VerticalLayout {

    private static final String FILTER_ALL = "ALL";
    private static final String DEFAULT_CUSTOM_SAMPLE = """
            Agent: Thank you for calling Harbor Cloud support, this is Maya.
            Customer: Hi Maya, hope your morning is going well! I was charged twice on my corporate card for invoice #HC-4490 and need the duplicate $299 charge refunded immediately. I clicked submit once and the spinner timed out before charging us twice.
            """;

    private final VejDaemonManager daemonManager;
    private final EvaluationComparisonService evaluationService;

    private final List<ComparisonRow> allRows = new ArrayList<>();
    private final ListDataProvider<ComparisonRow> dataProvider = new ListDataProvider<>(allRows);
    private final ExecutorService runExecutor = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "vej-benchmark-runner");
        t.setDaemon(true);
        return t;
    });

    private final AtomicBoolean batchRunning = new AtomicBoolean(false);
    private volatile AtomicBoolean activeCancelFlag = new AtomicBoolean(false);
    private volatile boolean syncingModelSelect = false;
    private Consumer<VejDaemonManager.DaemonSnapshot> daemonListener;

    // Connection & credentials controls
    private final Badge vejStatusBadge = new Badge("Checking...");
    private final TextField vejUrlField = new TextField("Local vej URL");
    private final Select<String> vejModelSelect = new Select<>();
    private final Map<String, String> modelDisplayLabels = new LinkedHashMap<>();
    private final Button startVejBtn = new Button("Start Local vej", VaadinIcon.PLAY.create());
    private final Button stopVejBtn = new Button("Stop Managed", VaadinIcon.STOP.create());
    private final Button refreshVejBtn = new Button("Refresh", VaadinIcon.REFRESH.create());
    private final Button logsBtn = new Button("Logs", VaadinIcon.TERMINAL.create());

    private final Badge jevModeBadge = new Badge("vej-only");
    private final PasswordField jevTokenField = new PasswordField("Jev API Token (Optional)");
    private final TextField jevModelField = new TextField("Jev Model");
    private final TextField jevUrlField = new TextField("Jev Endpoint");

    // Benchmark controls
    private final Select<ExtractionMode> extractionModeSelect = new Select<>();
    private final Select<String> categoryFilterSelect = new Select<>();
    private final Switch secondaryQuestionsSwitch = new Switch(
            "Also score Urgency (0-5) & Escalation (Yes/No)", true);
    private final Button runAllBtn = new Button("Run All 25 Examples", VaadinIcon.BOLT.create());
    private final Button stopRunBtn = new Button("Stop Run", VaadinIcon.STOP.create());
    private final Button clearResultsBtn = new Button("Reset Results", VaadinIcon.TRASH.create());
    private final ProgressBar progressBar = new ProgressBar(0, 1, 0);
    private final Span progressLabel = new Span("Ready — 25 creative examples loaded across 5 categories.");

    // Custom tester
    private final TextArea customInputArea = new TextArea("Customer Service Interaction Text");
    private final Span customExtractedPreview = new Span();
    private final Span customCharsSavedBadge = new Span();
    private final Button evaluateCustomBtn = new Button("Evaluate Custom Interaction", VaadinIcon.PLAY_CIRCLE.create());
    private final HorizontalLayout customQuickResult = new HorizontalLayout();

    // KPI cards
    private final KpiCard kpiAgreement = new KpiCard("Completed & Agreement");
    private final KpiCard kpiVejAccuracy = new KpiCard("Local vej Accuracy");
    private final KpiCard kpiVejSpeed = new KpiCard("Local vej Speed");
    private final KpiCard kpiJevStats = new KpiCard("Hosted Jev");
    private final KpiCard kpiSavings = new KpiCard("Speedup & Token Trim");

    private final Grid<ComparisonRow> grid = new Grid<>(ComparisonRow.class, false);
    private final DetailInspectionPanel detailPanel = new DetailInspectionPanel();

    public MainView(
            VejDaemonManager daemonManager,
            EvaluationComparisonService evaluationService,
            @Value("${vej.base-url:http://127.0.0.1:8080}") String defaultVejBaseUrl,
            @Value("${vej.jev-model:jev-latest}") String defaultJevModel) {
        this.daemonManager = daemonManager;
        this.evaluationService = evaluationService;

        setSizeUndefined();
        setWidthFull();
        addClassName("demo-page");

        initRows();
        add(buildHeader());
        add(buildConnectionGrid(defaultVejBaseUrl, defaultJevModel));
        add(buildCustomTesterSection());
        add(buildControlsAndProgress());
        add(buildKpiStrip());
        add(buildGrid());
        add(detailPanel);

        detailPanel.setOnLoadIntoTester(text -> {
            customInputArea.setValue(text);
            updateCustomPreview(text);
            Notification.show("Loaded transcript into Custom Tester", 2000, Notification.Position.BOTTOM_END);
        });

        applySnapshot(daemonManager.getSnapshot());
        updateJevModeBadge();
        updateSummaryCards();
        if (!allRows.isEmpty()) {
            grid.select(allRows.get(0));
        }
    }

    @Override
    protected void onAttach(AttachEvent attachEvent) {
        super.onAttach(attachEvent);
        UI ui = attachEvent.getUI();
        daemonListener = snapshot -> {
            if (ui.isAttached()) {
                ui.access(() -> applySnapshot(snapshot));
            }
        };
        daemonManager.addListener(daemonListener);
        applySnapshot(daemonManager.getSnapshot());
    }

    @Override
    protected void onDetach(DetachEvent detachEvent) {
        if (daemonListener != null) {
            daemonManager.removeListener(daemonListener);
            daemonListener = null;
        }
        activeCancelFlag.set(true);
        super.onDetach(detachEvent);
    }

    private void initRows() {
        allRows.clear();
        for (CallExample ex : CustomerServiceExamples.all()) {
            var ext = FirstSentenceExtractor.extract(ex.transcript());
            allRows.add(new ComparisonRow(
                    ex,
                    ext.firstReasonSentence(),
                    ext.startOffset(),
                    ext.endOffset(),
                    ExtractionMode.FIRST_REASON_SENTENCE));
        }
    }

    private VerticalLayout buildHeader() {
        VerticalLayout header = new VerticalLayout();
        header.setPadding(false);
        header.setSpacing(false);
        header.setWidthFull();

        HorizontalLayout titleRow = new HorizontalLayout();
        titleRow.setWidthFull();
        titleRow.addClassName("hero-header");
        titleRow.setPadding(false);

        VerticalLayout titles = new VerticalLayout();
        titles.setPadding(false);
        titles.setSpacing(false);
        H2 title = new H2("Customer Service Call Categorization — vej vs. Jev");
        title.getStyle().set("margin", "0");
        Span subtitle = new Span(
                "Extracts the caller's first reason-for-call sentence from multi-turn interactions and benchmarks local single-pass vej against hosted TypeSafe AI Jev.");
        subtitle.addClassName("hero-subtitle");
        titles.add(title, subtitle);

        HorizontalLayout categoryLegend = new HorizontalLayout();
        categoryLegend.setPadding(false);
        categoryLegend.setSpacing(true);
        categoryLegend.getStyle().set("flex-wrap", "wrap");
        for (CallCategory category : CallCategory.values()) {
            categoryLegend.add(CategoryBadgeFactory.categoryBadge(category));
        }

        titleRow.add(titles, categoryLegend);
        header.add(titleRow);
        return header;
    }

    private Div buildConnectionGrid(String defaultVejBaseUrl, String defaultJevModel) {
        Div gridContainer = new Div();
        gridContainer.addClassName("config-grid");

        // --- Local vej card ---
        Card vejCard = new Card();
        vejCard.addThemeVariants(CardVariant.OUTLINED);
        vejCard.setTitle("Local vej Engine");
        vejCard.setSubtitle("On-device single-forward-pass inference");
        vejCard.setHeaderSuffix(vejStatusBadge);

        vejUrlField.setValue(defaultVejBaseUrl);
        vejUrlField.setWidth("220px");

        vejModelSelect.setLabel("Local Model");
        vejModelSelect.setWidth("230px");
        vejModelSelect.setItemLabelGenerator(id -> modelDisplayLabels.getOrDefault(id, id));
        vejModelSelect.addValueChangeListener(e -> {
            if (syncingModelSelect || e.getValue() == null || e.getValue().isBlank()) return;
            handleModelSwitch(e.getValue());
        });

        HorizontalLayout vejFields = new HorizontalLayout(vejUrlField, vejModelSelect);
        vejFields.setPadding(false);
        vejFields.setWidthFull();
        vejFields.getStyle().set("flex-wrap", "wrap");

        startVejBtn.addThemeVariants(ButtonVariant.SMALL, ButtonVariant.PRIMARY);
        startVejBtn.addClickListener(e -> {
            startVejBtn.setEnabled(false);
            String requestedModel = vejModelSelect.getValue();
            daemonManager.startManagedDaemonAsync(vejUrlField.getValue(), requestedModel);
        });

        stopVejBtn.addThemeVariants(ButtonVariant.SMALL, ButtonVariant.ERROR);
        stopVejBtn.addClickListener(e -> daemonManager.stopManagedDaemon());

        refreshVejBtn.addThemeVariants(ButtonVariant.SMALL);
        refreshVejBtn.addClickListener(e -> daemonManager.refreshStatus(vejUrlField.getValue()));

        logsBtn.addThemeVariants(ButtonVariant.SMALL, ButtonVariant.TERTIARY);
        logsBtn.addClickListener(e -> openDaemonLogsDialog());

        HorizontalLayout vejButtons = new HorizontalLayout(startVejBtn, stopVejBtn, refreshVejBtn, logsBtn);
        vejButtons.setPadding(false);
        vejButtons.getStyle().set("flex-wrap", "wrap");

        vejCard.add(vejFields, vejButtons);

        // --- Hosted Jev card ---
        Card jevCard = new Card();
        jevCard.addThemeVariants(CardVariant.OUTLINED);
        jevCard.setTitle("Hosted Jev (TypeSafe AI)");
        jevCard.setSubtitle("Cloud baseline using the identical questions payload");
        jevCard.setHeaderSuffix(jevModeBadge);

        String envToken = evaluationService.configuredJevApiKey();
        if (envToken == null || envToken.isBlank()) {
            envToken = System.getenv("TYPESAFE_API_KEY");
        }
        if (envToken != null && !envToken.isBlank()) {
            jevTokenField.setValue(envToken.trim());
        }
        jevTokenField.setPlaceholder("Paste TYPESAFE_API_KEY (kept in memory only)");
        jevTokenField.setRevealButtonVisible(true);
        jevTokenField.setClearButtonVisible(true);
        jevTokenField.setValueChangeMode(ValueChangeMode.EAGER);
        jevTokenField.setWidthFull();
        jevTokenField.setHelperText("Leave blank to benchmark local vej alone; enter token to compare vej vs. Jev side-by-side.");
        jevTokenField.addValueChangeListener(e -> updateJevModeBadge());

        jevModelField.setValue(defaultJevModel);
        jevModelField.setWidth("170px");

        jevUrlField.setValue(VejClient.HOSTED_BASE_URL);
        jevUrlField.setWidth("240px");

        HorizontalLayout jevAdvanced = new HorizontalLayout(jevModelField, jevUrlField);
        jevAdvanced.setPadding(false);
        jevAdvanced.setWidthFull();
        jevAdvanced.getStyle().set("flex-wrap", "wrap");

        jevCard.add(jevTokenField, jevAdvanced);

        gridContainer.add(vejCard, jevCard);
        return gridContainer;
    }

    private Card buildCustomTesterSection() {
        Card testerCard = new Card();
        testerCard.addThemeVariants(CardVariant.OUTLINED);
        testerCard.setWidthFull();
        testerCard.setTitle("Live Custom Transcript Tester");
        testerCard.setSubtitle("Type or paste any customer interaction to preview first-sentence extraction and test vej vs. Jev immediately.");

        customInputArea.setWidthFull();
        customInputArea.setMinRows(3);
        customInputArea.setMaxRows(6);
        customInputArea.setValueChangeMode(ValueChangeMode.EAGER);
        customInputArea.setValue(DEFAULT_CUSTOM_SAMPLE.trim());
        customInputArea.addValueChangeListener(e -> updateCustomPreview(e.getValue()));

        Div previewBox = new Div();
        previewBox.addClassName("extracted-preview-box");
        HorizontalLayout previewHeader = new HorizontalLayout();
        previewHeader.setPadding(false);
        previewHeader.setWidthFull();
        previewHeader.setJustifyContentMode(FlexComponent.JustifyContentMode.BETWEEN);
        Span previewTitle = new Span("Extracted First Reason Sentence");
        previewTitle.getStyle().set("font-size", "0.8rem").set("font-weight", "600");
        customCharsSavedBadge.getStyle().set("font-size", "0.8rem").set("color", "var(--vaadin-text-color-secondary)");
        previewHeader.add(previewTitle, customCharsSavedBadge);
        customExtractedPreview.getStyle().set("display", "block").set("margin-top", "0.25rem").set("font-weight", "500");
        previewBox.add(previewHeader, customExtractedPreview);

        updateCustomPreview(customInputArea.getValue());

        evaluateCustomBtn.addThemeVariants(ButtonVariant.PRIMARY);
        evaluateCustomBtn.addClickListener(e -> evaluateCustomText());

        customQuickResult.setPadding(false);
        customQuickResult.setAlignItems(FlexComponent.Alignment.CENTER);
        customQuickResult.getStyle().set("flex-wrap", "wrap");

        HorizontalLayout actionRow = new HorizontalLayout(evaluateCustomBtn, customQuickResult);
        actionRow.setPadding(false);
        actionRow.setWidthFull();
        actionRow.setAlignItems(FlexComponent.Alignment.CENTER);
        actionRow.getStyle().set("flex-wrap", "wrap");

        Details testerDetails = new Details("Expand / Collapse Custom Tester", new VerticalLayout(
                customInputArea, previewBox, actionRow));
        testerDetails.setOpened(true);
        testerDetails.setWidthFull();
        testerCard.add(testerDetails);
        return testerCard;
    }

    private Card buildControlsAndProgress() {
        Card controlsCard = new Card();
        controlsCard.addThemeVariants(CardVariant.OUTLINED);
        controlsCard.setWidthFull();

        extractionModeSelect.setLabel("Evaluation Input Mode");
        extractionModeSelect.setItems(ExtractionMode.values());
        extractionModeSelect.setItemLabelGenerator(ExtractionMode::label);
        extractionModeSelect.setValue(ExtractionMode.FIRST_REASON_SENTENCE);
        extractionModeSelect.setWidth("270px");
        extractionModeSelect.addValueChangeListener(e -> {
            ExtractionMode mode = e.getValue() == null ? ExtractionMode.FIRST_REASON_SENTENCE : e.getValue();
            for (ComparisonRow row : allRows) {
                var ext = FirstSentenceExtractor.extract(row.example().transcript());
                row.setExtractedPreview(ext.firstReasonSentence(), ext.startOffset(), ext.endOffset(), mode);
            }
            dataProvider.refreshAll();
            ComparisonRow selected = grid.asSingleSelect().getValue();
            if (selected != null) {
                detailPanel.showRow(selected);
            }
            updateSummaryCards();
        });

        categoryFilterSelect.setLabel("Category Filter");
        List<String> filterItems = new ArrayList<>();
        filterItems.add(FILTER_ALL);
        for (CallCategory c : CallCategory.values()) {
            filterItems.add(c.key());
        }
        categoryFilterSelect.setItems(filterItems);
        categoryFilterSelect.setItemLabelGenerator(key -> FILTER_ALL.equals(key)
                ? "All 25 Examples"
                : CallCategory.fromKey(key).map(CallCategory::displayName).orElse(key) + " (5)");
        categoryFilterSelect.setValue(FILTER_ALL);
        categoryFilterSelect.setWidth("210px");
        categoryFilterSelect.addValueChangeListener(e -> applyFilter());

        secondaryQuestionsSwitch.setHelperText("Adds score + noul alongside category");

        runAllBtn.addThemeVariants(ButtonVariant.PRIMARY);
        runAllBtn.addClickListener(e -> startBatchRun());

        stopRunBtn.addThemeVariants(ButtonVariant.ERROR);
        stopRunBtn.setEnabled(false);
        stopRunBtn.addClickListener(e -> {
            activeCancelFlag.set(true);
            stopRunBtn.setEnabled(false);
            progressLabel.setText("Stopping after current example...");
        });

        clearResultsBtn.addThemeVariants(ButtonVariant.TERTIARY);
        clearResultsBtn.addClickListener(e -> clearAllResults());

        Div controlsRow = new Div();
        controlsRow.addClassName("controls-row");
        controlsRow.add(
                extractionModeSelect,
                categoryFilterSelect,
                secondaryQuestionsSwitch,
                runAllBtn,
                stopRunBtn,
                clearResultsBtn);

        progressBar.setWidthFull();
        progressLabel.getStyle().set("font-size", "0.85rem").set("color", "var(--vaadin-text-color-secondary)");

        VerticalLayout wrapper = new VerticalLayout(controlsRow, progressBar, progressLabel);
        wrapper.setPadding(false);
        wrapper.setSpacing(true);
        controlsCard.add(wrapper);
        return controlsCard;
    }

    private Div buildKpiStrip() {
        Div strip = new Div();
        strip.addClassName("kpi-strip");
        strip.add(
                kpiAgreement.card,
                kpiVejAccuracy.card,
                kpiVejSpeed.card,
                kpiJevStats.card,
                kpiSavings.card);
        return strip;
    }

    private Grid<ComparisonRow> buildGrid() {
        grid.setDataProvider(dataProvider);
        grid.setWidthFull();
        grid.setHeight("470px");
        grid.addThemeVariants(GridVariant.ROW_STRIPES, GridVariant.WRAP_CELL_CONTENT);
        grid.setSelectionMode(Grid.SelectionMode.SINGLE);

        grid.addColumn(row -> row.example().number())
                .setHeader("#")
                .setWidth("54px")
                .setFlexGrow(0);

        grid.addComponentColumn(row -> {
            VerticalLayout cell = new VerticalLayout();
            cell.setPadding(false);
            cell.setSpacing(false);
            Span idSpan = new Span(row.example().id());
            idSpan.getStyle().set("font-weight", "600");
            Span channelSpan = new Span(row.example().channel());
            channelSpan.getStyle().set("font-size", "0.78rem").set("color", "var(--vaadin-text-color-secondary)");
            cell.add(idSpan, channelSpan);
            return cell;
        }).setHeader("Call").setWidth("90px").setFlexGrow(0);

        grid.addComponentColumn(row -> CategoryBadgeFactory.categoryBadge(row.example().expectedCategory()))
                .setHeader("Expected")
                .setWidth("165px")
                .setFlexGrow(0);

        grid.addComponentColumn(row -> {
            VerticalLayout cell = new VerticalLayout();
            cell.setPadding(false);
            cell.setSpacing(false);
            Span text = new Span(row.evaluatedText());
            text.getStyle().set("font-size", "0.88rem");
            double savedPct = row.charReductionRatio() * 100.0;
            String note = row.extractionMode() == ExtractionMode.FIRST_REASON_SENTENCE
                    ? String.format(Locale.US, "Extracted 1st sentence (%.0f%% shorter than full transcript)", savedPct)
                    : "Full transcript";
            Span sub = new Span(note);
            sub.getStyle().set("font-size", "0.75rem").set("color", "var(--vaadin-text-color-secondary)");
            cell.add(text, sub);
            return cell;
        }).setHeader("Evaluated Text").setFlexGrow(2).setWidth("320px");

        grid.addComponentColumn(row -> {
            if (row.runState() == ComparisonRow.RunState.RUNNING && row.vejOutcome() == null) {
                Badge running = new Badge("Running...");
                running.addThemeVariants(BadgeVariant.SMALL);
                return running;
            }
            return CategoryBadgeFactory.outcomeCell(row.vejOutcome(), row.example().expectedCategory());
        }).setHeader("Local vej").setWidth("185px").setFlexGrow(0);

        grid.addComponentColumn(row -> formatTimingCell(row.vejOutcome(), true))
                .setHeader("vej Time")
                .setWidth("130px")
                .setFlexGrow(0);

        grid.addComponentColumn(row -> {
            if (row.runState() == ComparisonRow.RunState.RUNNING && row.jevOutcome() == null) {
                Badge running = new Badge("Running...");
                running.addThemeVariants(BadgeVariant.SMALL);
                return running;
            }
            return CategoryBadgeFactory.outcomeCell(row.jevOutcome(), row.example().expectedCategory());
        }).setHeader("Hosted Jev").setWidth("225px").setFlexGrow(0);

        grid.addComponentColumn(row -> formatTimingCell(row.jevOutcome(), false))
                .setHeader("Jev Time")
                .setWidth("130px")
                .setFlexGrow(0);

        grid.addComponentColumn(row -> CategoryBadgeFactory.agreementBadge(
                row.hasBothSuccesses(), row.enginesAgree()))
                .setHeader("Match")
                .setWidth("92px")
                .setFlexGrow(0);

        grid.addComponentColumn(row -> {
            Button evalOne = new Button(VaadinIcon.PLAY.create());
            evalOne.addThemeVariants(ButtonVariant.SMALL, ButtonVariant.TERTIARY);
            evalOne.getElement().setAttribute("title", "Evaluate this interaction");
            evalOne.setEnabled(!batchRunning.get());
            evalOne.addClickListener(e -> evaluateSingleRow(row));
            return evalOne;
        }).setHeader("Run").setWidth("68px").setFlexGrow(0);

        grid.asSingleSelect().addValueChangeListener(e -> {
            if (e.getValue() != null) {
                detailPanel.showRow(e.getValue());
            }
        });

        return grid;
    }

    private VerticalLayout formatTimingCell(EngineOutcome outcome, boolean showServerLatency) {
        VerticalLayout cell = new VerticalLayout();
        cell.setPadding(false);
        cell.setSpacing(false);
        if (outcome == null || outcome.skipped()) {
            Span dash = new Span("—");
            dash.getStyle().set("color", "var(--vaadin-text-color-secondary)");
            cell.add(dash);
            return cell;
        }
        if (outcome.errorMessage() != null) {
            Span err = new Span(String.format(Locale.US, "%.0f ms", outcome.clientLatencyMs()));
            err.addClassName("mono-metric");
            err.getStyle().set("color", "var(--aura-red-text)");
            cell.add(err);
            return cell;
        }
        Span client = new Span(String.format(Locale.US, "%.0f ms", outcome.clientLatencyMs()));
        client.addClassName("mono-metric");
        client.getStyle().set("font-weight", "600");
        String detailText = showServerLatency && outcome.serverLatencyMs() != null
                ? String.format(Locale.US, "srv %.0f ms • %d/%d tok",
                        outcome.serverLatencyMs(), outcome.inputTokens(), outcome.outputTokens())
                : String.format(Locale.US, "%d in / %d out tok",
                        outcome.inputTokens(), outcome.outputTokens());
        Span detail = new Span(detailText);
        detail.addClassName("mono-metric");
        detail.getStyle().set("font-size", "0.73rem").set("color", "var(--vaadin-text-color-secondary)");
        cell.add(client, detail);
        return cell;
    }

    private void applyFilter() {
        String filter = categoryFilterSelect.getValue();
        if (filter == null || FILTER_ALL.equals(filter)) {
            dataProvider.clearFilters();
            runAllBtn.setText("Run All 25 Examples");
        } else {
            dataProvider.setFilter(row -> row.example().expectedCategory() != null
                    && row.example().expectedCategory().key().equals(filter));
            long count = allRows.stream()
                    .filter(r -> r.example().expectedCategory().key().equals(filter))
                    .count();
            runAllBtn.setText("Run Filtered (" + count + ")");
        }
        updateSummaryCards();
    }

    private List<ComparisonRow> currentVisibleRows() {
        String filter = categoryFilterSelect.getValue();
        if (filter == null || FILTER_ALL.equals(filter)) {
            return List.copyOf(allRows);
        }
        return allRows.stream()
                .filter(r -> r.example().expectedCategory().key().equals(filter))
                .toList();
    }

    private void startBatchRun() {
        if (!batchRunning.compareAndSet(false, true)) {
            return;
        }
        List<ComparisonRow> targetRows = currentVisibleRows();
        if (targetRows.isEmpty()) {
            batchRunning.set(false);
            return;
        }

        UI ui = UI.getCurrent();
        AtomicBoolean cancelFlag = new AtomicBoolean(false);
        activeCancelFlag = cancelFlag;

        String vejBaseUrl = vejUrlField.getValue();
        String vejModel = vejModelSelect.getValue();
        String jevBaseUrl = jevUrlField.getValue();
        String jevToken = jevTokenField.getValue();
        String jevModel = jevModelField.getValue();
        boolean includeSecondary = Boolean.TRUE.equals(secondaryQuestionsSwitch.getValue());
        ExtractionMode mode = extractionModeSelect.getValue() == null
                ? ExtractionMode.FIRST_REASON_SENTENCE
                : extractionModeSelect.getValue();

        runAllBtn.setEnabled(false);
        stopRunBtn.setEnabled(true);
        clearResultsBtn.setEnabled(false);
        progressBar.setMin(0);
        progressBar.setMax(targetRows.size());
        progressBar.setValue(0);
        progressLabel.setText("Starting evaluation of " + targetRows.size() + " interactions...");

        runExecutor.submit(() -> {
            int done = 0;
            boolean surfacedJevError = false;
            for (ComparisonRow row : targetRows) {
                if (cancelFlag.get()) {
                    break;
                }
                var ext = FirstSentenceExtractor.extract(row.example().transcript());
                row.setExtractedPreview(ext.firstReasonSentence(), ext.startOffset(), ext.endOffset(), mode);
                row.setRunState(ComparisonRow.RunState.RUNNING);
                if (ui.isAttached()) {
                    ui.access(() -> dataProvider.refreshItem(row));
                }

                EvaluationComparisonService.ComparisonPair pair = evaluationService.compare(
                        row.evaluatedText(),
                        vejBaseUrl,
                        vejModel,
                        jevBaseUrl,
                        jevToken,
                        jevModel,
                        includeSecondary,
                        1.0);

                row.setVejOutcome(pair.vejOutcome());
                row.setJevOutcome(pair.jevOutcome());
                row.setRunState(ComparisonRow.RunState.DONE);
                done++;

                String firstBatchJevError = null;
                if (!surfacedJevError && pair.jevOutcome() != null && pair.jevOutcome().errorMessage() != null) {
                    surfacedJevError = true;
                    firstBatchJevError = pair.jevOutcome().errorMessage();
                }

                int finishedCount = done;
                String notifyErr = firstBatchJevError;
                String statusLine = buildRowProgressText(finishedCount, targetRows.size(), row);
                if (ui.isAttached()) {
                    ui.access(() -> {
                        dataProvider.refreshItem(row);
                        progressBar.setValue(finishedCount);
                        progressLabel.setText(statusLine);
                        updateSummaryCards();
                        if (notifyErr != null) {
                            Notification errN = Notification.show(
                                    "Hosted Jev error: " + CategoryBadgeFactory.truncate(notifyErr, 120),
                                    5000,
                                    Notification.Position.BOTTOM_END);
                            errN.addThemeVariants(NotificationVariant.ERROR);
                        }
                        ComparisonRow selected = grid.asSingleSelect().getValue();
                        if (selected == row) {
                            detailPanel.showRow(row);
                        }
                    });
                }
            }

            boolean wasCancelled = cancelFlag.get();
            int finalDone = done;
            batchRunning.set(false);
            if (ui.isAttached()) {
                ui.access(() -> {
                    runAllBtn.setEnabled(true);
                    stopRunBtn.setEnabled(false);
                    clearResultsBtn.setEnabled(true);
                    dataProvider.refreshAll();
                    updateSummaryCards();
                    String msg = wasCancelled
                            ? "Stopped after " + finalDone + " / " + targetRows.size() + " interactions."
                            : "Completed " + finalDone + " / " + targetRows.size() + " interactions.";
                    progressLabel.setText(msg);
                    Notification n = Notification.show(msg, 3000, Notification.Position.BOTTOM_END);
                    n.addThemeVariants(wasCancelled ? NotificationVariant.WARNING : NotificationVariant.SUCCESS);
                });
            }
        });
    }

    private void evaluateSingleRow(ComparisonRow row) {
        if (batchRunning.get()) return;
        UI ui = UI.getCurrent();
        String vejBaseUrl = vejUrlField.getValue();
        String vejModel = vejModelSelect.getValue();
        String jevBaseUrl = jevUrlField.getValue();
        String jevToken = jevTokenField.getValue();
        String jevModel = jevModelField.getValue();
        boolean includeSecondary = Boolean.TRUE.equals(secondaryQuestionsSwitch.getValue());
        ExtractionMode mode = extractionModeSelect.getValue() == null
                ? ExtractionMode.FIRST_REASON_SENTENCE
                : extractionModeSelect.getValue();

        var ext = FirstSentenceExtractor.extract(row.example().transcript());
        row.setExtractedPreview(ext.firstReasonSentence(), ext.startOffset(), ext.endOffset(), mode);
        row.setRunState(ComparisonRow.RunState.RUNNING);
        dataProvider.refreshItem(row);
        grid.select(row);

        runExecutor.submit(() -> {
            var pair = evaluationService.compare(
                    row.evaluatedText(),
                    vejBaseUrl,
                    vejModel,
                    jevBaseUrl,
                    jevToken,
                    jevModel,
                    includeSecondary,
                    1.0);
            row.setVejOutcome(pair.vejOutcome());
            row.setJevOutcome(pair.jevOutcome());
            row.setRunState(ComparisonRow.RunState.DONE);
            if (ui.isAttached()) {
                ui.access(() -> {
                    dataProvider.refreshItem(row);
                    detailPanel.showRow(row);
                    updateSummaryCards();
                });
            }
        });
    }

    private void evaluateCustomText() {
        String raw = customInputArea.getValue();
        if (raw == null || raw.isBlank()) {
            Notification.show("Enter a customer service interaction first.", 2500, Notification.Position.BOTTOM_END)
                    .addThemeVariants(NotificationVariant.WARNING);
            return;
        }
        UI ui = UI.getCurrent();
        ExtractionMode mode = extractionModeSelect.getValue() == null
                ? ExtractionMode.FIRST_REASON_SENTENCE
                : extractionModeSelect.getValue();
        var ext = FirstSentenceExtractor.extract(raw);
        String textToSend = mode == ExtractionMode.FIRST_REASON_SENTENCE
                ? ext.firstReasonSentence()
                : raw;

        String vejBaseUrl = vejUrlField.getValue();
        String vejModel = vejModelSelect.getValue();
        String jevBaseUrl = jevUrlField.getValue();
        String jevToken = jevTokenField.getValue();
        String jevModel = jevModelField.getValue();
        boolean includeSecondary = Boolean.TRUE.equals(secondaryQuestionsSwitch.getValue());

        evaluateCustomBtn.setEnabled(false);
        customQuickResult.removeAll();
        customQuickResult.add(new Span("Evaluating..."));

        runExecutor.submit(() -> {
            var pair = evaluationService.compare(
                    textToSend,
                    vejBaseUrl,
                    vejModel,
                    jevBaseUrl,
                    jevToken,
                    jevModel,
                    includeSecondary,
                    1.0);
            CallExample customEx = new CallExample(0, "CUSTOM", null, "Custom Input", "Interactive User", raw);
            ComparisonRow customRow = new ComparisonRow(
                    customEx, ext.firstReasonSentence(), ext.startOffset(), ext.endOffset(), mode);
            customRow.setVejOutcome(pair.vejOutcome());
            customRow.setJevOutcome(pair.jevOutcome());
            customRow.setRunState(ComparisonRow.RunState.DONE);

            if (ui.isAttached()) {
                ui.access(() -> {
                    evaluateCustomBtn.setEnabled(true);
                    customQuickResult.removeAll();
                    Span vejLbl = new Span("vej:");
                    vejLbl.getStyle().set("font-weight", "600");
                    customQuickResult.add(vejLbl, CategoryBadgeFactory.outcomeCell(pair.vejOutcome(), null));
                    if (pair.vejOutcome().isSuccess()) {
                        Span t = new Span(String.format(Locale.US, "(%.0f ms)", pair.vejOutcome().clientLatencyMs()));
                        t.addClassName("mono-metric");
                        customQuickResult.add(t);
                    }
                    Span jevLbl = new Span("• Jev:");
                    jevLbl.getStyle().set("font-weight", "600");
                    customQuickResult.add(jevLbl, CategoryBadgeFactory.outcomeCell(pair.jevOutcome(), null));
                    if (pair.jevOutcome().isSuccess()) {
                        Span t = new Span(String.format(Locale.US, "(%.0f ms)", pair.jevOutcome().clientLatencyMs()));
                        t.addClassName("mono-metric");
                        customQuickResult.add(t);
                    } else if (pair.jevOutcome().errorMessage() != null) {
                        Notification errN = Notification.show(
                                "Hosted Jev error: " + CategoryBadgeFactory.truncate(pair.jevOutcome().errorMessage(), 140),
                                5000,
                                Notification.Position.BOTTOM_END);
                        errN.addThemeVariants(NotificationVariant.ERROR);
                    }
                    grid.deselectAll();
                    detailPanel.showRow(customRow);
                });
            }
        });
    }

    private void clearAllResults() {
        if (batchRunning.get()) return;
        for (ComparisonRow row : allRows) {
            row.setVejOutcome(null);
            row.setJevOutcome(null);
            row.setRunState(ComparisonRow.RunState.PENDING);
        }
        progressBar.setValue(0);
        progressLabel.setText("Cleared results — ready to evaluate.");
        dataProvider.refreshAll();
        updateSummaryCards();
        ComparisonRow selected = grid.asSingleSelect().getValue();
        if (selected != null) {
            detailPanel.showRow(selected);
        }
    }

    private void updateCustomPreview(String raw) {
        var ext = FirstSentenceExtractor.extract(raw);
        if (ext.firstReasonSentence().isEmpty()) {
            customExtractedPreview.setText("(Enter text above to preview the extracted sentence)");
            customCharsSavedBadge.setText("");
            return;
        }
        customExtractedPreview.setText("\"" + ext.firstReasonSentence() + "\"");
        int fullLen = raw == null ? 0 : raw.length();
        int sentLen = ext.firstReasonSentence().length();
        double savedPct = fullLen > 0 && sentLen < fullLen
                ? ((double) (fullLen - sentLen) / fullLen) * 100.0
                : 0.0;
        customCharsSavedBadge.setText(String.format(
                Locale.US, "%d / %d chars (%.0f%% reduction)", sentLen, fullLen, savedPct));
    }

    private void applySnapshot(VejDaemonManager.DaemonSnapshot snapshot) {
        if (snapshot == null) return;
        vejStatusBadge.removeThemeVariants(
                BadgeVariant.SUCCESS, BadgeVariant.WARNING, BadgeVariant.ERROR, BadgeVariant.CONTRAST);

        switch (snapshot.phase()) {
            case CONNECTED -> {
                vejStatusBadge.setText("Connected • " + snapshot.activeModel());
                vejStatusBadge.addThemeVariants(BadgeVariant.SUCCESS);
                startVejBtn.setEnabled(false);
                stopVejBtn.setEnabled(snapshot.managedByApp());
            }
            case STARTING -> {
                vejStatusBadge.setText("Starting...");
                vejStatusBadge.addThemeVariants(BadgeVariant.WARNING);
                startVejBtn.setEnabled(false);
                stopVejBtn.setEnabled(true);
            }
            case ERROR -> {
                vejStatusBadge.setText("Error");
                vejStatusBadge.addThemeVariants(BadgeVariant.ERROR);
                startVejBtn.setEnabled(true);
                stopVejBtn.setEnabled(snapshot.managedByApp());
            }
            case STOPPED -> {
                vejStatusBadge.setText("Offline");
                vejStatusBadge.addThemeVariants(BadgeVariant.CONTRAST);
                startVejBtn.setEnabled(true);
                stopVejBtn.setEnabled(false);
            }
        }
        vejStatusBadge.getElement().setAttribute("title", snapshot.statusMessage());

        syncingModelSelect = true;
        try {
            modelDisplayLabels.clear();
            List<String> ids = new ArrayList<>();
            for (ModelInfo m : snapshot.models()) {
                if (m.isDownloaded() || m.isActive()) {
                    ids.add(m.id());
                    String label = String.format(Locale.US, "%s (%.1f GB)%s",
                            m.id(), m.sizeGb(), m.isActive() ? " • active" : "");
                    modelDisplayLabels.put(m.id(), label);
                }
            }
            if (ids.isEmpty() && snapshot.activeModel() != null && !snapshot.activeModel().isBlank()) {
                ids.add(snapshot.activeModel());
                modelDisplayLabels.put(snapshot.activeModel(), snapshot.activeModel());
            }
            vejModelSelect.setItems(ids);
            if (!snapshot.activeModel().isBlank() && ids.contains(snapshot.activeModel())) {
                vejModelSelect.setValue(snapshot.activeModel());
            } else if (!ids.isEmpty() && vejModelSelect.getValue() == null) {
                vejModelSelect.setValue(ids.get(0));
            }
            vejModelSelect.setEnabled(!ids.isEmpty());
        } finally {
            syncingModelSelect = false;
        }
    }

    private void handleModelSwitch(String targetModel) {
        UI ui = UI.getCurrent();
        String baseUrl = vejUrlField.getValue();
        vejModelSelect.setEnabled(false);
        Notification.show("Switching local vej model to " + targetModel + "...", 2000, Notification.Position.BOTTOM_END);
        runExecutor.submit(() -> {
            try {
                String switched = evaluationService.switchLocalModel(baseUrl, targetModel);
                daemonManager.refreshStatus(baseUrl);
                if (ui.isAttached()) {
                    ui.access(() -> {
                        vejModelSelect.setEnabled(true);
                        Notification.show("Switched local vej to " + switched, 2500, Notification.Position.BOTTOM_END)
                                .addThemeVariants(NotificationVariant.SUCCESS);
                    });
                }
            } catch (VejException ex) {
                if (ui.isAttached()) {
                    ui.access(() -> {
                        vejModelSelect.setEnabled(true);
                        Notification.show("Model switch failed: " + ex.getMessage(), 4000, Notification.Position.BOTTOM_END)
                                .addThemeVariants(NotificationVariant.ERROR);
                    });
                }
            }
        });
    }

    private void updateJevModeBadge() {
        boolean hasToken = jevTokenField.getValue() != null && !jevTokenField.getValue().isBlank();
        jevModeBadge.removeThemeVariants(BadgeVariant.SUCCESS, BadgeVariant.CONTRAST);
        if (hasToken) {
            jevModeBadge.setText("Dual Compare Active");
            jevModeBadge.addThemeVariants(BadgeVariant.SUCCESS);
        } else {
            jevModeBadge.setText("vej-only (No Token)");
            jevModeBadge.addThemeVariants(BadgeVariant.CONTRAST);
        }
    }

    private void updateSummaryCards() {
        List<ComparisonRow> rows = currentVisibleRows();
        BenchmarkSummary s = BenchmarkSummary.compute(rows);

        String agreementText = s.comparableBothCount() == 0
                ? String.format(Locale.US, "%d / %d", s.completedRows(), s.totalRows())
                : String.format(Locale.US, "%d / %d  (%.0f%% agree)", s.completedRows(), s.totalRows(), s.agreementPct());
        kpiAgreement.set(
                agreementText,
                s.comparableBothCount() == 0
                        ? "Run with a Jev token to measure agreement"
                        : s.agreementCount() + " of " + s.comparableBothCount() + " matched Jev");

        kpiVejAccuracy.set(
                s.expectedEvaluatedVejCount() == 0 ? "—" : String.format(Locale.US, "%.1f%%", s.vejAccuracyPct()),
                s.expectedEvaluatedVejCount() == 0
                        ? "Matches vs. expected labels"
                        : String.format(Locale.US, "%d/%d correct • avg conf %.1f%%",
                                s.vejCorrectCount(), s.expectedEvaluatedVejCount(), s.vejAvgConfidence() * 100.0));

        kpiVejSpeed.set(
                s.vejSuccessCount() == 0 ? "—" : String.format(Locale.US, "%.0f ms", s.vejAvgClientMs()),
                s.vejSuccessCount() == 0
                        ? "0 output tokens (single prefill pass)"
                        : String.format(Locale.US, "Engine %.0f ms • %.0f in / %.0f out tok",
                                s.vejAvgServerMs(), s.vejAvgInputTokens(), s.vejAvgOutputTokens()));

        if (s.jevSuccessCount() == 0 && s.jevErrorCount() > 0) {
            kpiJevStats.set(
                    s.jevErrorCount() + " err",
                    CategoryBadgeFactory.truncate(s.firstJevError(), 70));
        } else if (s.jevSuccessCount() == 0) {
            boolean hasToken = jevTokenField.getValue() != null && !jevTokenField.getValue().isBlank();
            kpiJevStats.set("—", hasToken ? "Click Run to evaluate via Jev" : "Enter Jev token above to compare");
        } else {
            kpiJevStats.set(
                    String.format(Locale.US, "%.0f ms", s.jevAvgClientMs()),
                    String.format(Locale.US, "%.1f%% accurate (%d/%d) • %.0f in / %.0f out tok",
                            s.jevAccuracyPct(), s.jevCorrectCount(), s.expectedEvaluatedJevCount(),
                            s.jevAvgInputTokens(), s.jevAvgOutputTokens()));
        }

        String speedupValue = s.speedupFactor() > 0
                ? String.format(Locale.US, "%.2fx faster", s.speedupFactor())
                : String.format(Locale.US, "%.0f%% text trim", s.avgCharReductionPct());
        String speedupDetail = s.speedupFactor() > 0
                ? String.format(Locale.US, "vej vs Jev • 1st-sentence cut %.0f%% chars", s.avgCharReductionPct())
                : "Avg character savings from 1st-sentence extraction";
        kpiSavings.set(speedupValue, speedupDetail);
    }

    private void openDaemonLogsDialog() {
        Dialog dialog = new Dialog();
        dialog.setHeaderTitle("Local vej Daemon Diagnostics");
        dialog.setWidth("760px");

        VejDaemonManager.DaemonSnapshot snap = daemonManager.getSnapshot();
        Div info = new Div();
        info.getStyle().set("margin-bottom", "var(--vaadin-gap-s)").set("font-size", "0.88rem");
        info.setText("Binary: " + (snap.binaryPath().isBlank() ? "not found" : snap.binaryPath())
                + " | Status: " + snap.statusMessage());

        Div logBox = new Div();
        logBox.addClassName("log-box");
        List<String> lines = daemonManager.getRecentLogs();
        logBox.setText(lines.isEmpty() ? "(No managed process logs yet)" : String.join("\n", lines));

        Button close = new Button("Close", e -> dialog.close());
        dialog.add(info, logBox);
        dialog.getFooter().add(close);
        dialog.open();
    }

    private static String buildRowProgressText(int done, int total, ComparisonRow row) {
        EngineOutcome vej = row.vejOutcome();
        EngineOutcome jev = row.jevOutcome();
        String vejStr = vej != null && vej.isSuccess()
                ? String.format(Locale.US, "vej=%s (%.0f ms)", vej.rawCategoryChoice(), vej.clientLatencyMs())
                : "vej=" + (vej != null && vej.errorMessage() != null ? "err" : "n/a");
        String jevStr = jev != null && jev.isSuccess()
                ? String.format(Locale.US, "Jev=%s (%.0f ms)", jev.rawCategoryChoice(), jev.clientLatencyMs())
                : (jev != null && jev.skipped() ? "Jev=skipped" : "Jev=err");
        return String.format(Locale.US, "Evaluated %d / %d • %s: %s | %s",
                done, total, row.example().id(), vejStr, jevStr);
    }

    private static final class KpiCard {
        final Card card = new Card();
        final Span value = new Span("—");
        final Span detail = new Span("");

        KpiCard(String title) {
            card.addThemeVariants(CardVariant.OUTLINED);
            card.setTitle(title);
            value.addClassName("kpi-value");
            detail.addClassName("kpi-detail");
            VerticalLayout content = new VerticalLayout(value, detail);
            content.setPadding(false);
            content.setSpacing(false);
            card.add(content);
        }

        void set(String valueText, String detailText) {
            value.setText(valueText);
            detail.setText(detailText);
        }
    }
}

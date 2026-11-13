import { PRESETS } from './presets.js';
import { escapeHtml as esc, modelOptionsFrom } from './html.js';

const API_KEY_STORAGE_KEY = 'typesafe_api_key';

/** Storage access can throw (private windows, blocked site data); never let it break the UI. */
function storageGet(storage, key) {
  try {
    return storage.getItem(key);
  } catch (_) {
    return null;
  }
}

function storageSet(storage, key, value) {
  try {
    storage.setItem(key, value);
  } catch (_) {
    // Persistence is a convenience only.
  }
}

function storageRemove(storage, key) {
  try {
    storage.removeItem(key);
  } catch (_) {
    // Nothing to clean up if storage is unavailable.
  }
}

class VejPlayground {
  constructor() {
    this.target = storageGet(localStorage, 'vej_target') || 'vej';
    // The TypeSafe API key lives in memory. It is persisted only when the user opts in, and
    // then only in sessionStorage (this tab). Older versions kept it in localStorage, readable
    // by anything later served from the same origin, so remove that copy.
    storageRemove(localStorage, API_KEY_STORAGE_KEY);
    this.apiKey = storageGet(sessionStorage, API_KEY_STORAGE_KEY) || '';
    this.vejModelOptions = modelOptionsFrom(null);
    this.activePresetId = 'support-triage';
    this.leftTab = 'builder';
    this.rightTab = 'verdict';
    this.codeLang = 'curl';

    this.state = '';
    this.questions = {};
    this.temperature = 0.2;
    this.model = 'llama-3.2-1b';
    this.stateMode = 'text'; // 'text' | 'json'
    this.questionsMode = 'builder'; // 'builder' | 'json'

    this.isRunning = false;
    this.results = null; // { vej: ..., jev: ... }

    this.initElements();
    this.loadPreset(this.activePresetId);
    this.bindEvents();
    this.checkDaemonHealth();
    this.loadModelCatalog();
  }

  initElements() {
    this.targetSelect = document.getElementById('target-select');
    this.modelSelect = document.getElementById('model-select');
    this.apiKeyGroup = document.getElementById('api-key-group');
    this.apiKeyInput = document.getElementById('api-key-input');
    this.rememberKeyCheckbox = document.getElementById('remember-key-checkbox');
    this.daemonStatus = document.getElementById('daemon-status');
    this.presetSelect = document.getElementById('preset-select');
    this.presetDesc = document.getElementById('preset-desc');
    this.resetPresetBtn = document.getElementById('reset-preset-btn');

    this.stateTextarea = document.getElementById('state-input');
    this.stateTabNav = document.getElementById('state-tab-nav');
    this.stateJsonError = document.getElementById('state-json-error');
    this.formatStateBtn = document.getElementById('format-state-btn');

    this.questionsContainer = document.getElementById('questions-container');
    this.questionsTabNav = document.getElementById('questions-tab-nav');
    this.questionsJsonView = document.getElementById('questions-json-view');
    this.questionsJsonEditor = document.getElementById('questions-json-editor');
    this.questionsJsonError = document.getElementById('questions-json-error');
    this.addQuestionBtn = document.getElementById('add-question-btn');
    this.formatQuestionsJsonBtn = document.getElementById('format-questions-json-btn');

    this.jsonEditor = document.getElementById('raw-json-editor');
    this.tempSlider = document.getElementById('temp-slider');
    this.tempVal = document.getElementById('temp-val');

    this.runBtn = document.getElementById('run-btn');
    this.resultsContainer = document.getElementById('results-container');
    this.toastEl = document.getElementById('toast');

    // Set saved target & key
    this.targetSelect.value = this.target;
    this.apiKeyInput.value = this.apiKey;
    if (this.rememberKeyCheckbox) {
      this.rememberKeyCheckbox.checked = this.apiKey !== '';
    }
    this.updateModelOptions();
    const savedModel = storageGet(localStorage, 'vej_model');
    if (savedModel && Array.from(this.modelSelect.options).some(o => o.value === savedModel)) {
      this.model = savedModel;
      this.modelSelect.value = savedModel;
    }
    this.updateTargetVisibility();
    this.renderPresets();
  }

  renderPresets() {
    if (!this.presetSelect) return;
    this.presetSelect.innerHTML = PRESETS.map(p => `
      <option value="${esc(p.id)}" ${p.id === this.activePresetId ? 'selected' : ''}>
        ${esc(p.icon)} ${esc(p.name)}
      </option>
    `).join('');
    this.updatePresetDescription();
  }

  /** Replaces the model <select> options. Built with DOM nodes, so names are never parsed as HTML. */
  renderModelSelect(options) {
    const fragment = document.createDocumentFragment();
    for (const opt of options) {
      const el = document.createElement('option');
      el.value = opt.value;
      el.textContent = opt.label;
      el.disabled = opt.disabled;
      fragment.appendChild(el);
    }
    this.modelSelect.replaceChildren(fragment);
  }

  /** Fills the vej model list from GET /v1/models, keeping the catalog fallback when offline. */
  async loadModelCatalog() {
    try {
      const url = window.location.protocol === 'file:' ? 'http://127.0.0.1:8080/v1/models' : '/v1/models';
      const res = await fetch(url, { signal: AbortSignal.timeout(1500) });
      if (!res.ok) return;
      const options = modelOptionsFrom(await res.json());
      if (options.length === 0) return;
      this.vejModelOptions = options;
    } catch (_) {
      return; // Daemon offline: the catalog fallback stays in place.
    }
    if (this.target === 'jev') return;

    const hasSaved = storageGet(localStorage, 'vej_model') !== null;
    const active = this.vejModelOptions.find(o => o.active);
    if (!hasSaved && active) {
      this.model = active.value;
    }
    this.updateModelOptions();
    this.syncToRawJson();
    this.renderCodeExport();
  }

  updatePresetDescription() {
    if (!this.presetDesc) return;
    const preset = PRESETS.find(p => p.id === this.activePresetId);
    this.presetDesc.textContent = preset ? preset.description : '';
  }

  loadPreset(id) {
    const preset = PRESETS.find(p => p.id === id);
    if (!preset) return;

    this.activePresetId = id;
    this.state = preset.data.state;
    this.questions = JSON.parse(JSON.stringify(preset.data.questions));
    this.temperature = preset.data.temperature || 0.8;
    // Preserve user's currently selected model; do not override when changing examples
    if (this.target === 'jev') {
      if (!this.model || !this.model.startsWith('jev')) {
        this.model = 'jev-latest';
      }
    } else {
      const active = this.modelSelect?.value || this.model || storageGet(localStorage, 'vej_model');
      if (active && !active.startsWith('jev')) {
        this.model = active;
      } else {
        this.model = preset.data.model || 'llama-3.2-1b';
      }
    }

    // Immediately update text and questions editor values so downstream sync/payload calls use the new preset
    this.stateTextarea.value = this.state;
    if (this.questionsJsonEditor) {
      this.questionsJsonEditor.value = JSON.stringify(this.questions, null, 2);
    }
    if (this.questionsJsonError) {
      this.questionsJsonError.style.display = 'none';
    }
    this.renderQuestionsBuilder();

    // Auto-detect whether state is structured JSON or plain text
    let isJsonState = false;
    try {
      const trimmed = (preset.data.state || '').trim();
      if ((trimmed.startsWith('{') && trimmed.endsWith('}')) || (trimmed.startsWith('[') && trimmed.endsWith(']'))) {
        JSON.parse(trimmed);
        isJsonState = true;
      }
    } catch (_) {}
    this.setStateMode(isJsonState ? 'json' : 'text');

    this.tempSlider.value = this.temperature;
    this.tempVal.textContent = this.temperature;
    if (this.modelSelect && this.modelSelect.value !== this.model) {
      this.modelSelect.value = this.model;
    }
    if (this.presetSelect && this.presetSelect.value !== id) {
      this.presetSelect.value = id;
    }
    this.updatePresetDescription();

    // Clear stale decision results when loading a new preset
    this.results = null;
    this.renderResults();

    this.syncToRawJson();
    this.renderCodeExport();
  }

  bindEvents() {
    // Target change
    this.targetSelect.addEventListener('change', (e) => {
      this.target = e.target.value;
      storageSet(localStorage, 'vej_target', this.target);
      this.updateTargetVisibility();
      this.updateModelOptions();
      this.renderCodeExport();
    });

    // API Key input: kept in memory; saved to sessionStorage only when "remember" is checked.
    this.apiKeyInput.addEventListener('input', (e) => {
      this.apiKey = e.target.value;
      this.persistApiKey();
    });

    if (this.rememberKeyCheckbox) {
      this.rememberKeyCheckbox.addEventListener('change', () => this.persistApiKey());
    }

    // Model change
    this.modelSelect.addEventListener('change', (e) => {
      this.model = e.target.value;
      if (!this.model.startsWith('jev')) {
        storageSet(localStorage, 'vej_model', this.model);
      }
      this.syncToRawJson();
      this.renderCodeExport();
    });

    // Preset select change
    if (this.presetSelect) {
      this.presetSelect.addEventListener('change', (e) => {
        this.loadPreset(e.target.value);
      });
    }

    // Reset preset button
    if (this.resetPresetBtn) {
      this.resetPresetBtn.addEventListener('click', () => {
        this.loadPreset(this.activePresetId);
        this.showToast('Reset example to defaults');
      });
    }

    // State mode tabs (Text vs JSON)
    if (this.stateTabNav) {
      this.stateTabNav.querySelectorAll('[data-state-mode]').forEach(btn => {
        btn.addEventListener('click', () => {
          this.setStateMode(btn.dataset.stateMode);
        });
      });
    }

    // State text changes
    this.stateTextarea.addEventListener('input', () => {
      this.state = this.stateTextarea.value;
      if (this.stateMode === 'json') {
        this.validateStateJson();
      }
      this.syncToRawJson();
      this.renderCodeExport();
    });

    // Format State JSON button
    if (this.formatStateBtn) {
      this.formatStateBtn.addEventListener('click', () => {
        try {
          const parsed = JSON.parse(this.stateTextarea.value);
          this.state = JSON.stringify(parsed, null, 2);
          this.stateTextarea.value = this.state;
          if (this.stateJsonError) this.stateJsonError.style.display = 'none';
          this.syncToRawJson();
          this.renderCodeExport();
          this.showToast('State JSON formatted');
        } catch (e) {
          this.showToast('State is not valid JSON: ' + e.message, true);
        }
      });
    }

    // Questions mode tabs (Visual vs JSON)
    if (this.questionsTabNav) {
      this.questionsTabNav.querySelectorAll('[data-questions-mode]').forEach(btn => {
        btn.addEventListener('click', () => {
          this.setQuestionsMode(btn.dataset.questionsMode);
        });
      });
    }

    // Questions JSON Editor changes
    if (this.questionsJsonEditor) {
      this.questionsJsonEditor.addEventListener('input', () => {
        if (this.validateQuestionsJson()) {
          this.syncToRawJson();
          this.renderCodeExport();
        }
      });
    }

    // Format Questions JSON button
    if (this.formatQuestionsJsonBtn) {
      this.formatQuestionsJsonBtn.addEventListener('click', () => {
        try {
          const parsed = JSON.parse(this.questionsJsonEditor.value);
          if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
            throw new Error('Questions must be a JSON object mapping names to question definitions');
          }
          this.questions = parsed;
          this.questionsJsonEditor.value = JSON.stringify(parsed, null, 2);
          if (this.questionsJsonError) this.questionsJsonError.style.display = 'none';
          this.syncToRawJson();
          this.renderCodeExport();
          this.showToast('Questions JSON formatted');
        } catch (e) {
          this.showToast('Questions JSON invalid: ' + e.message, true);
        }
      });
    }

    // Add Question
    if (this.addQuestionBtn) {
      this.addQuestionBtn.addEventListener('click', () => {
        const qIndex = Object.keys(this.questions).length + 1;
        const qName = `question_${qIndex}`;
        this.questions[qName] = {
          type: 'choice',
          instructions: 'Choose the best matching category',
          criteria: {
            option_a: 'First criteria description',
            option_b: 'Second criteria description'
          }
        };
        this.renderQuestionsBuilder();
        if (this.questionsJsonEditor) {
          this.questionsJsonEditor.value = JSON.stringify(this.questions, null, 2);
        }
        this.syncToRawJson();
        this.renderCodeExport();
      });
    }

    // Temperature slider
    this.tempSlider.addEventListener('input', (e) => {
      this.temperature = parseFloat(e.target.value);
      this.tempVal.textContent = this.temperature;
      this.syncToRawJson();
      this.renderCodeExport();
    });

    // Left Tab navigation (Builder vs Raw JSON)
    document.querySelectorAll('[data-left-tab]').forEach(btn => {
      btn.addEventListener('click', (e) => {
        document.querySelectorAll('[data-left-tab]').forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
        this.leftTab = btn.dataset.leftTab;

        if (this.leftTab === 'builder') {
          document.getElementById('builder-view').style.display = 'flex';
          document.getElementById('raw-json-view').style.display = 'none';
          this.syncFromRawJson();
          this.renderQuestionsBuilder();
        } else {
          document.getElementById('builder-view').style.display = 'none';
          document.getElementById('raw-json-view').style.display = 'flex';
          this.syncToRawJson();
        }
      });
    });

    // Raw JSON input
    this.jsonEditor.addEventListener('input', () => {
      this.syncFromRawJson();
      this.renderCodeExport();
    });

    // Right Tab navigation
    document.querySelectorAll('[data-right-tab]').forEach(btn => {
      btn.addEventListener('click', () => {
        document.querySelectorAll('[data-right-tab]').forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
        this.rightTab = btn.dataset.rightTab;
        this.renderResults();
      });
    });

    // Code lang selector
    document.querySelectorAll('[data-code-lang]').forEach(btn => {
      btn.addEventListener('click', () => {
        document.querySelectorAll('[data-code-lang]').forEach(b => b.classList.remove('active'));
        btn.classList.add('active');
        this.codeLang = btn.dataset.codeLang;
        this.renderCodeExport();
      });
    });

    // Copy code button
    document.getElementById('copy-code-btn').addEventListener('click', () => {
      const code = document.getElementById('code-output').textContent;
      navigator.clipboard.writeText(code);
      this.showToast('Code copied to clipboard!');
    });

    // Run button & keyboard shortcut
    this.runBtn.addEventListener('click', () => this.executeDecision());
    window.addEventListener('keydown', (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        e.preventDefault();
        this.executeDecision();
      }
    });
  }

  persistApiKey() {
    const remember = this.rememberKeyCheckbox?.checked === true;
    if (remember && this.apiKey) {
      storageSet(sessionStorage, API_KEY_STORAGE_KEY, this.apiKey);
    } else {
      storageRemove(sessionStorage, API_KEY_STORAGE_KEY);
    }
  }

  updateTargetVisibility() {
    if (this.target === 'jev' || this.target === 'dual') {
      this.apiKeyGroup.style.display = 'flex';
    } else {
      this.apiKeyGroup.style.display = 'none';
    }
  }

  updateModelOptions() {
    if (this.target === 'jev') {
      this.renderModelSelect([
        { value: 'jev-latest', label: 'jev-latest', disabled: false },
        { value: 'jev-1.13.0', label: 'jev-1.13.0', disabled: false },
      ]);
    } else {
      this.renderModelSelect(this.vejModelOptions);
    }
    const selectable = Array.from(this.modelSelect.options).filter(o => !o.disabled);
    if (this.model && selectable.some(o => o.value === this.model)) {
      this.modelSelect.value = this.model;
    } else if (selectable.length > 0) {
      this.model = selectable[0].value;
      this.modelSelect.value = this.model;
    }
  }

  setStateMode(mode) {
    this.stateMode = mode;
    if (this.stateTabNav) {
      this.stateTabNav.querySelectorAll('[data-state-mode]').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.stateMode === mode);
      });
    }

    if (mode === 'json') {
      if (this.formatStateBtn) this.formatStateBtn.style.display = 'inline-block';
      this.validateStateJson();
    } else {
      if (this.formatStateBtn) this.formatStateBtn.style.display = 'none';
      if (this.stateJsonError) this.stateJsonError.style.display = 'none';
    }
    this.syncToRawJson();
    this.renderCodeExport();
  }

  validateStateJson() {
    if (this.stateMode !== 'json' || !this.stateJsonError) return true;
    const raw = this.stateTextarea.value.trim();
    if (!raw) {
      this.stateJsonError.style.display = 'none';
      return true;
    }
    try {
      JSON.parse(raw);
      this.stateJsonError.style.display = 'none';
      return true;
    } catch (err) {
      this.stateJsonError.textContent = `Invalid JSON: ${err.message}`;
      this.stateJsonError.style.display = 'block';
      return false;
    }
  }

  setQuestionsMode(mode) {
    if (mode === 'builder' && this.questionsMode === 'json') {
      const raw = this.questionsJsonEditor.value.trim();
      if (raw) {
        try {
          const parsed = JSON.parse(raw);
          if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
            throw new Error('Questions must be an object (e.g. { "question_name": { "type": "choice", ... } })');
          }
          this.questions = parsed;
          this.renderQuestionsBuilder();
        } catch (err) {
          this.showToast(`Cannot switch to Visual mode: ${err.message}`, true);
          return;
        }
      }
    }

    this.questionsMode = mode;
    if (this.questionsTabNav) {
      this.questionsTabNav.querySelectorAll('[data-questions-mode]').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.questionsMode === mode);
      });
    }

    if (mode === 'json') {
      this.questionsContainer.style.display = 'none';
      this.addQuestionBtn.style.display = 'none';
      this.questionsJsonView.style.display = 'flex';
      this.formatQuestionsJsonBtn.style.display = 'inline-block';
      this.questionsJsonEditor.value = JSON.stringify(this.questions, null, 2);
      this.validateQuestionsJson();
    } else {
      this.questionsContainer.style.display = 'flex';
      this.addQuestionBtn.style.display = 'inline-block';
      this.questionsJsonView.style.display = 'none';
      this.formatQuestionsJsonBtn.style.display = 'none';
      if (this.questionsJsonError) this.questionsJsonError.style.display = 'none';
    }
    this.syncToRawJson();
    this.renderCodeExport();
  }

  validateQuestionsJson() {
    if (this.questionsMode !== 'json' || !this.questionsJsonError) return true;
    const raw = this.questionsJsonEditor.value.trim();
    if (!raw) {
      this.questionsJsonError.textContent = 'Questions JSON cannot be empty';
      this.questionsJsonError.style.display = 'block';
      return false;
    }
    try {
      const parsed = JSON.parse(raw);
      if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
        throw new Error('Questions must be an object mapping names to question definitions');
      }
      this.questions = parsed;
      this.questionsJsonError.style.display = 'none';
      return true;
    } catch (err) {
      this.questionsJsonError.textContent = `Invalid Questions JSON: ${err.message}`;
      this.questionsJsonError.style.display = 'block';
      return false;
    }
  }

  renderQuestionsBuilder() {
    this.questionsContainer.innerHTML = Object.entries(this.questions).map(([qName, qSpec]) => {
      const isChoice = qSpec.type === 'choice';
      const isNoul = qSpec.type === 'noul';
      const isScore = qSpec.type === 'score';
      
      const crit = qSpec.criteria || qSpec.options || qSpec.choices || qSpec.levels;
      let criteriaEntries = [];
      if (Array.isArray(crit)) {
        criteriaEntries = crit.map((v, i) => [String(i), v]);
      } else if (crit && typeof crit === 'object' && Object.keys(crit).length > 0) {
        criteriaEntries = Object.entries(crit);
      } else if (isNoul) {
        criteriaEntries = [['true', 'True (yes)'], ['false', 'False (no)']];
      }

      let instrDisplay = '';
      if (typeof qSpec.instructions === 'string') {
        instrDisplay = qSpec.instructions;
      } else if (typeof qSpec.instructions === 'object' && qSpec.instructions !== null) {
        instrDisplay = qSpec.instructions.question || JSON.stringify(qSpec.instructions);
      }

      return `
        <div class="question-card" data-qname="${esc(qName)}">
          <div class="question-card-header">
            <input class="q-input" style="max-width: 140px; font-weight: 700;" value="${esc(qName)}" placeholder="Question Name" data-action="rename-q" />
            <select class="control-select q-input" style="max-width: 110px;" data-action="change-type">
              <option value="choice" ${isChoice ? 'selected' : ''}>Choice</option>
              <option value="noul" ${isNoul ? 'selected' : ''}>Noul (Yes/No)</option>
              <option value="score" ${isScore ? 'selected' : ''}>Score</option>
            </select>
            <button class="remove-btn" title="Delete Question" data-action="delete-q">&times;</button>
          </div>

          <div class="field-group">
            <label class="field-label">Instructions / Question</label>
            <input class="q-input" value="${esc(instrDisplay)}" placeholder="e.g. Which team should handle this ticket?" data-action="update-instr" />
          </div>

          <div class="field-group">
            <div class="field-header">
              <label class="field-label">Criteria & Options</label>
              ${isChoice || isScore ? `<button class="action-link" data-action="add-crit-row">+ Add Option</button>` : ''}
            </div>
            
            <div class="criteria-list">
              ${criteriaEntries.map(([k, v]) => `
                <div class="criteria-row" data-key="${esc(k)}">
                  <input class="q-input" value="${esc(k)}" ${isNoul ? 'readonly' : ''} placeholder="Key (e.g. billing or 0)" data-crit="key" />
                  <input class="q-input" value="${esc(v)}" placeholder="Description / criteria rule..." data-crit="val" />
                  ${!isNoul ? `<button class="remove-btn" data-action="delete-crit-row">&times;</button>` : '<div></div>'}
                </div>
              `).join('')}
            </div>
          </div>
        </div>
      `;
    }).join('');

    // Attach question builder event listeners
    this.questionsContainer.querySelectorAll('.question-card').forEach(card => {
      const qName = card.dataset.qname;

      card.querySelector('[data-action="rename-q"]').addEventListener('change', (e) => {
        const newName = e.target.value.trim();
        if (newName && newName !== qName) {
          const spec = this.questions[qName];
          delete this.questions[qName];
          this.questions[newName] = spec;
          this.renderQuestionsBuilder();
          this.syncToRawJson();
          this.renderCodeExport();
        }
      });

      card.querySelector('[data-action="change-type"]').addEventListener('change', (e) => {
        const newType = e.target.value;
        this.questions[qName].type = newType;
        if (newType === 'noul') {
          this.questions[qName].criteria = {
            true: 'Condition is met / Yes',
            false: 'Condition is not met / No'
          };
        }
        this.renderQuestionsBuilder();
        this.syncToRawJson();
        this.renderCodeExport();
      });

      card.querySelector('[data-action="delete-q"]').addEventListener('click', () => {
        delete this.questions[qName];
        this.renderQuestionsBuilder();
        this.syncToRawJson();
        this.renderCodeExport();
      });

      card.querySelector('[data-action="update-instr"]').addEventListener('input', (e) => {
        this.questions[qName].instructions = e.target.value;
        this.syncToRawJson();
        this.renderCodeExport();
      });

      const addCritBtn = card.querySelector('[data-action="add-crit-row"]');
      if (addCritBtn) {
        addCritBtn.addEventListener('click', () => {
          if (!this.questions[qName].criteria) {
            this.questions[qName].criteria = {};
          }
          if (Array.isArray(this.questions[qName].criteria)) {
            this.questions[qName].criteria.push(`Level ${this.questions[qName].criteria.length}`);
          } else {
            const count = Object.keys(this.questions[qName].criteria || {}).length + 1;
            this.questions[qName].criteria[`option_${count}`] = `Description for option ${count}`;
          }
          this.renderQuestionsBuilder();
          this.syncToRawJson();
          this.renderCodeExport();
        });
      }

      card.querySelectorAll('.criteria-row').forEach(row => {
        const origKey = row.dataset.key;
        const keyInput = row.querySelector('[data-crit="key"]');
        const valInput = row.querySelector('[data-crit="val"]');
        const delBtn = row.querySelector('[data-action="delete-crit-row"]');

        keyInput.addEventListener('change', (e) => {
          const newK = e.target.value.trim();
          if (newK && newK !== origKey) {
            if (!this.questions[qName].criteria) {
              this.questions[qName].criteria = {};
            }
            if (Array.isArray(this.questions[qName].criteria)) {
              const obj = {};
              this.questions[qName].criteria.forEach((val, idx) => {
                obj[String(idx)] = val;
              });
              this.questions[qName].criteria = obj;
            }
            const val = this.questions[qName].criteria[origKey];
            delete this.questions[qName].criteria[origKey];
            this.questions[qName].criteria[newK] = val;
            this.syncToRawJson();
            this.renderCodeExport();
          }
        });

        valInput.addEventListener('input', (e) => {
          if (!this.questions[qName].criteria) {
            this.questions[qName].criteria = {};
          }
          if (Array.isArray(this.questions[qName].criteria)) {
            const idx = parseInt(origKey, 10);
            if (!isNaN(idx)) {
              this.questions[qName].criteria[idx] = e.target.value;
            }
          } else {
            this.questions[qName].criteria[origKey] = e.target.value;
          }
          this.syncToRawJson();
          this.renderCodeExport();
        });

        if (delBtn) {
          delBtn.addEventListener('click', () => {
            if (Array.isArray(this.questions[qName].criteria)) {
              const idx = parseInt(origKey, 10);
              if (!isNaN(idx)) {
                this.questions[qName].criteria.splice(idx, 1);
              }
            } else if (this.questions[qName].criteria) {
              delete this.questions[qName].criteria[origKey];
            }
            this.renderQuestionsBuilder();
            this.syncToRawJson();
            this.renderCodeExport();
          });
        }
      });
    });
  }

  buildPayload() {
    let parsedState = this.state;
    if (this.stateMode === 'json') {
      try {
        parsedState = JSON.parse(this.state);
      } catch (_) {
        // keep string
      }
    } else {
      parsedState = String(this.state);
    }

    let currentQuestions = this.questions;
    if (this.questionsMode === 'json' && this.questionsJsonEditor) {
      try {
        const parsed = JSON.parse(this.questionsJsonEditor.value);
        if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) {
          currentQuestions = parsed;
        }
      } catch (_) {}
    }

    return {
      model: this.model,
      state: parsedState,
      questions: currentQuestions,
      temperature: this.temperature
    };
  }

  syncToRawJson() {
    const payload = this.buildPayload();
    this.jsonEditor.value = JSON.stringify(payload, null, 2);
  }

  syncFromRawJson() {
    try {
      const payload = JSON.parse(this.jsonEditor.value);
      if (payload.state !== undefined) {
        if (typeof payload.state === 'string') {
          this.state = payload.state;
          try {
            const trimmed = payload.state.trim();
            if ((trimmed.startsWith('{') && trimmed.endsWith('}')) || (trimmed.startsWith('[') && trimmed.endsWith(']'))) {
              JSON.parse(trimmed);
              this.setStateMode('json');
            } else {
              this.setStateMode('text');
            }
          } catch (_) {}
        } else {
          this.state = JSON.stringify(payload.state, null, 2);
          this.setStateMode('json');
        }
        this.stateTextarea.value = this.state;
      }
      if (payload.questions) {
        this.questions = payload.questions;
        if (this.questionsJsonEditor) {
          this.questionsJsonEditor.value = JSON.stringify(this.questions, null, 2);
        }
        if (this.questionsMode === 'builder') {
          this.renderQuestionsBuilder();
        }
      }
      if (payload.temperature !== undefined) {
        this.temperature = payload.temperature;
        this.tempSlider.value = this.temperature;
        this.tempVal.textContent = this.temperature;
      }
      if (payload.model) {
        this.model = payload.model;
        this.modelSelect.value = this.model;
      }
    } catch (_) {
      // invalid JSON during typing, ignore
    }
  }

  async checkDaemonHealth() {
    try {
      const url = window.location.protocol === 'file:' ? 'http://127.0.0.1:8080/health' : '/health';
      const res = await fetch(url, { signal: AbortSignal.timeout(1500) });
      if (res.ok) {
        const data = await res.json();
        this.daemonStatus.className = 'status-indicator';
        this.daemonStatus.innerHTML = `<span class="status-dot"></span> vej daemon active (${esc(data.active_model)})`;
      } else {
        throw new Error('Non-200');
      }
    } catch (_) {
      this.daemonStatus.className = 'status-indicator offline';
      this.daemonStatus.innerHTML = `<span class="status-dot"></span> vej daemon offline (run 'vej serve')`;
    }
  }

  async executeDecision() {
    if (this.isRunning) return;

    if (this.stateMode === 'json' && !this.validateStateJson()) {
      this.showToast('Please fix Input State JSON syntax before running', true);
      return;
    }
    if (this.questionsMode === 'json' && !this.validateQuestionsJson()) {
      this.showToast('Please fix Questions JSON syntax before running', true);
      return;
    }

    this.isRunning = true;
    this.runBtn.disabled = true;
    this.runBtn.innerHTML = `<span>⏳</span> Evaluating...`;

    const payload = this.buildPayload();

    try {
      if (this.target === 'dual') {
        const jevModel = (this.model && this.model.startsWith('jev')) ? this.model : 'jev-latest';
        const vejModel = (!this.model || this.model.startsWith('jev')) ? 'llama-3.2-1b' : this.model;

        // Jev expects state as string, model, and questions
        const jevPayload = {
          model: jevModel,
          state: typeof payload.state === 'string' ? payload.state : JSON.stringify(payload.state),
          questions: payload.questions
        };

        // Dual parallel run against vej and Jev
        const [vejRes, jevRes] = await Promise.allSettled([
          this.callEndpoint('/v1/systemone', { ...payload, model: vejModel }, null),
          this.callEndpoint('/v1/proxy/jev', jevPayload, this.apiKey)
        ]);

        this.results = {
          vej: vejRes.status === 'fulfilled' ? vejRes.value : { error: vejRes.reason.message },
          jev: jevRes.status === 'fulfilled' ? jevRes.value : { error: jevRes.reason.message }
        };

        if (jevRes.status === 'rejected') {
          this.showToast(`Jev error: ${jevRes.reason.message}`, true);
        }
      } else if (this.target === 'jev') {
        const jevModel = (this.model && this.model.startsWith('jev')) ? this.model : 'jev-latest';
        const jevPayload = {
          model: jevModel,
          state: typeof payload.state === 'string' ? payload.state : JSON.stringify(payload.state),
          questions: payload.questions
        };
        const res = await this.callEndpoint('/v1/proxy/jev', jevPayload, this.apiKey);
        this.results = { jev: res };
      } else {
        const res = await this.callEndpoint('/v1/systemone', payload, null);
        this.results = { vej: res };
      }

      this.renderResults();
      if (this.target !== 'dual' || this.results.jev?.data) {
        this.showToast('Decision evaluated successfully!');
      }
    } catch (err) {
      this.showToast(`Error: ${err.message}`, true);
    } finally {
      this.isRunning = false;
      this.runBtn.disabled = false;
      this.runBtn.innerHTML = `<span>⚡</span> Run Decision`;
      this.checkDaemonHealth();
    }
  }

  async callEndpoint(endpoint, payload, apiKey) {
    const start = performance.now();
    const headers = { 'Content-Type': 'application/json' };
    if (apiKey) {
      headers['Authorization'] = `Bearer ${apiKey.trim()}`;
    }

    let url = endpoint;
    if (url.startsWith('/') && window.location.protocol === 'file:') {
      url = `http://127.0.0.1:8080${url}`;
    }

    const res = await fetch(url, {
      method: 'POST',
      headers,
      body: JSON.stringify(payload)
    });

    const elapsed = performance.now() - start;
    let body = {};
    const text = await res.text();
    try {
      body = JSON.parse(text);
    } catch (_) {
      body = { error: text };
    }

    if (!res.ok) {
      console.error('[API Error]', res.status, body);
      const errMsg = body.detail?.message || (typeof body.detail === 'string' ? body.detail : null) || body.error || body.message || (typeof body === 'string' ? body : text) || `HTTP ${res.status}`;
      throw new Error(typeof errMsg === 'object' ? JSON.stringify(errMsg) : errMsg);
    }

    return {
      data: body,
      latency_ms: elapsed,
      model: payload.model
    };
  }



  renderResults() {
    if (!this.results) {
      this.resultsContainer.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">🎯</div>
          <div class="empty-title">Ready for Decision</div>
          <div class="empty-desc">Hit <b>Run Decision</b> or press <b>⌘+Enter</b> to classify state with restricted softmax inference.</div>
        </div>
      `;
      return;
    }

    if (this.rightTab === 'json') {
      const displayData = this.target === 'dual' ? this.results : (this.results.vej?.data || this.results.jev?.data);
      this.resultsContainer.innerHTML = `
        <div class="code-container" style="flex: 1;">
          <div class="code-header">
            <span class="code-lang">Response JSON</span>
            <button class="copy-btn" id="copy-json-btn">Copy</button>
          </div>
          <pre class="code-content">${esc(JSON.stringify(displayData, null, 2))}</pre>
        </div>
      `;
      document.getElementById('copy-json-btn').addEventListener('click', () => {
        navigator.clipboard.writeText(JSON.stringify(displayData, null, 2));
        this.showToast('JSON copied to clipboard');
      });
      return;
    }

    if (this.target === 'dual') {
      this.renderDualResults();
    } else {
      const activeRes = this.results.vej || this.results.jev;
      this.renderSingleResults(activeRes, this.target.toUpperCase());
    }
  }

  getOptionLabel(qName, k) {
    const qSpec = this.questions?.[qName];
    if (!qSpec) return k;

    const crit = qSpec.criteria || qSpec.options || qSpec.choices || qSpec.levels;
    if (!crit) return k;

    // 1. Array criteria (e.g. score levels: ["None", "2 years", "4 years", "6 years", "8 years", "10+ years"])
    if (Array.isArray(crit)) {
      const idx = parseInt(k, 10);
      if (!isNaN(idx) && crit[idx] !== undefined) {
        const text = String(crit[idx]);
        return text.startsWith(`${idx}:`) ? text : `${idx}: ${text}`;
      }
    }

    // 2. Object criteria (e.g. { "steady_growth": "Promotions...", "0": "None" })
    if (typeof crit === 'object' && crit !== null) {
      if (crit[k] !== undefined) {
        const val = String(crit[k]);
        const isNumeric = !isNaN(parseInt(k, 10));
        if (isNumeric) {
          return `${k}: ${val}`;
        }
        if (val.length <= 40) {
          return `${k} (${val})`;
        }
        return k;
      }
    }

    return k;
  }

  getScoreLabelSub(qName, scoreVal) {
    const qSpec = this.questions?.[qName];
    if (!qSpec || typeof scoreVal !== 'number' || isNaN(scoreVal)) return '';

    const crit = qSpec.criteria || qSpec.options || qSpec.choices || qSpec.levels;
    if (!crit || !Array.isArray(crit)) return '';

    const rounded = Math.round(scoreVal);
    if (crit[rounded] !== undefined) {
      const text = String(crit[rounded]);
      return `≈ ${text.replace(/^\d+:\s*/, '')}`;
    }
    return '';
  }

  normalizeAnswer(answer, fallbackData = {}) {
    if (!answer) {
      if (fallbackData?.decision) {
        const prob = fallbackData.confidence || 1.0;
        return {
          type: 'choice',
          choice: fallbackData.decision,
          confidence: prob,
          probabilities: fallbackData.probabilities || { [fallbackData.decision]: prob }
        };
      }
      return null;
    }

    // Check if it's a Score primitive
    if (answer.score !== undefined || answer.type === 'score') {
      const scoreVal = typeof answer.score === 'number' ? answer.score : parseFloat(answer.score || answer.choice || 0);
      const probabilities = answer.probabilities || {};
      const keys = Object.keys(probabilities);
      const maxLevel = keys.length > 0 ? (keys.length - 1) : 5;
      const scoreStr = isNaN(scoreVal) ? (answer.choice || 'N/A') : `${scoreVal.toFixed(2)} of ${maxLevel}`;
      return {
        type: 'score',
        choice: scoreStr,
        score: scoreVal,
        maxLevel,
        confidence: answer.confidence !== undefined ? answer.confidence : 1.0,
        probabilities
      };
    }

    // Check if it's a Noul primitive
    if (answer.noul !== undefined || answer.type === 'noul') {
      let noulVal;
      if (typeof answer.noul === 'number') {
        noulVal = answer.noul;
      } else if (typeof answer.confidence === 'number') {
        noulVal = (answer.choice === 'false') ? (1 - answer.confidence) : answer.confidence;
      } else {
        noulVal = (answer.choice === 'true') ? 1.0 : ((answer.choice === 'false') ? 0.0 : 0.5);
      }

      noulVal = Math.max(0, Math.min(1, noulVal));
      const isTrue = noulVal >= 0.5;
      const confidence = isTrue ? noulVal : (1 - noulVal);

      const probabilities = (answer.probabilities && Object.keys(answer.probabilities).length > 0)
        ? answer.probabilities
        : {
            true: noulVal,
            false: parseFloat((1 - noulVal).toFixed(4))
          };

      const percent = (noulVal * 100).toFixed(0);
      return {
        type: 'noul',
        choice: `${percent}% true`,
        noul: noulVal,
        confidence: answer.confidence !== undefined ? answer.confidence : confidence,
        probabilities
      };
    }

    return {
      type: answer.type || 'choice',
      choice: answer.choice || fallbackData?.decision || 'N/A',
      confidence: answer.confidence ?? fallbackData?.confidence ?? 0,
      probabilities: answer.probabilities || fallbackData?.probabilities || {}
    };
  }

  renderQuestionHeader(qName, qSpec) {
    const qType = (qSpec?.type || 'choice').toUpperCase();
    const crit = qSpec?.criteria || qSpec?.options || qSpec?.choices || qSpec?.levels;
    let typeBadge = `<span class="badge" style="background: rgba(59, 130, 246, 0.15); color: #60a5fa; font-size: 0.7rem; padding: 2px 8px; border-radius: 4px; font-weight: 600;">${esc(qType)}</span>`;
    
    if (qType === 'SCORE') {
      const levels = Array.isArray(crit) ? crit.length : (typeof crit === 'object' && crit !== null ? Object.keys(crit).length : 6);
      typeBadge = `<span class="badge" style="background: rgba(16, 185, 129, 0.15); color: #34d399; font-size: 0.7rem; padding: 2px 8px; border-radius: 4px; font-weight: 600;">SCORE (${levels} levels)</span>`;
    } else if (qType === 'NOUL') {
      typeBadge = `<span class="badge" style="background: rgba(245, 158, 11, 0.15); color: #fbbf24; font-size: 0.7rem; padding: 2px 8px; border-radius: 4px; font-weight: 600;">NOUL</span>`;
    } else if (qType === 'CHOICE' && crit) {
      const optCount = Array.isArray(crit) ? crit.length : Object.keys(crit).length;
      typeBadge = `<span class="badge" style="background: rgba(59, 130, 246, 0.15); color: #60a5fa; font-size: 0.7rem; padding: 2px 8px; border-radius: 4px; font-weight: 600;">CHOICE (${optCount} options)</span>`;
    }

    let promptText = '';
    if (typeof qSpec?.instructions === 'string') {
      promptText = qSpec.instructions;
    } else if (typeof qSpec?.instructions === 'object' && qSpec?.instructions !== null) {
      promptText = qSpec.instructions.question || Object.entries(qSpec.instructions).map(([k, v]) => `${k}: ${v}`).join(' | ');
    }

    return `
      <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 0.5rem; margin-top: 1.25rem; border-top: 1px solid var(--border-color); padding-top: 1rem;">
        <div style="max-width: 80%;">
          <div style="font-weight: 700; font-size: 0.95rem; color: #f8fafc; font-family: monospace;">${esc(qName)}</div>
          ${promptText ? `<div style="font-size: 0.78rem; color: var(--text-muted); margin-top: 3px; line-height: 1.4;">${esc(promptText)}</div>` : ''}
        </div>
        ${typeBadge}
      </div>
    `;
  }

  renderSingleResults(res, label) {
    if (res.error) {
      this.resultsContainer.innerHTML = `
        <div class="verdict-hero" style="border-color: rgba(244, 63, 94, 0.4); background: rgba(244, 63, 94, 0.08);">
          <div class="verdict-badge" style="color: #fb7185;">Evaluation Error</div>
          <p style="font-size: 0.85rem; color: #fecdd3;">${esc(res.error)}</p>
        </div>
      `;
      return;
    }

    const { data, latency_ms } = res;
    const answers = data?.answers || {};
    const qNames = Object.keys(answers).length > 0 ? Object.keys(answers) : (Object.keys(this.questions).length > 0 ? Object.keys(this.questions) : ['decision']);

    let html = `
      <div class="verdict-hero">
        <div class="verdict-header">
          <div>
            <div style="font-size: 0.72rem; text-transform: uppercase; color: var(--text-muted); font-weight: 700;">${esc(label)} Evaluation</div>
            <div class="verdict-badge">${qNames.length} Question${qNames.length > 1 ? 's' : ''} Evaluated</div>
          </div>
          <div style="text-align: right;">
            <div style="font-size: 0.72rem; text-transform: uppercase; color: var(--text-muted); font-weight: 700;">Engine</div>
            <div class="confidence-meter">${esc(data?.model || 'unknown')}</div>
          </div>
        </div>

        <div class="pills-container">
          <span class="metric-pill">⏱️ Latency: ${(Number(data?.latency_ms || latency_ms) || 0).toFixed(1)} ms</span>
          <span class="metric-pill">📥 Input Tokens: ${esc(data?.usage?.input_tokens ?? data?.tokens_evaluated ?? 'N/A')}</span>
          <span class="metric-pill">⚡ Output Tokens: ${esc(data?.usage?.output_tokens ?? 0)}</span>
          <span class="metric-pill">🤖 Model: ${esc(data?.model || 'unknown')}</span>
        </div>
      </div>
    `;

    for (const qName of qNames) {
      const rawAnswer = answers[qName] || null;
      const answer = this.normalizeAnswer(rawAnswer, data) || {};
      const probs = answer.probabilities || {};
      const sortedProbs = Object.entries(probs).sort((a, b) => b[1] - a[1]);
      const qSpec = this.questions[qName] || {};
      const scoreSub = (answer.type === 'score') ? this.getScoreLabelSub(qName, answer.score) : '';

      html += this.renderQuestionHeader(qName, qSpec);
      html += `
        <div class="compare-card" style="margin-bottom: 0.75rem;">
          <div style="display: flex; justify-content: space-between; align-items: center;">
            <div>
              <div style="font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase;">Verdict</div>
              <div style="font-size: 1.25rem; font-weight: 800; color: #f8fafc;">${esc(answer.choice || 'N/A')}</div>
              ${scoreSub ? `<div style="font-size: 0.78rem; font-weight: 600; color: #93c5fd; margin-top: 2px;">${esc(scoreSub)}</div>` : ''}
            </div>
            <div style="text-align: right;">
              <div style="font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase;">Confidence</div>
              <div style="font-size: 1.1rem; font-weight: 800; color: #34d399;">${((answer.confidence || 0) * 100).toFixed(1)}%</div>
            </div>
          </div>
          <div class="dist-card" style="padding: 0.5rem; background: var(--bg-surface); margin-top: 0.5rem;">
            ${sortedProbs.map(([k, p], idx) => {
              const optLabel = this.getOptionLabel(qName, k);
              return `
                <div class="prob-row">
                  <div class="prob-meta" style="font-size: 0.75rem;">
                    <span>${idx === 0 ? '🏆 ' : ''}${esc(optLabel)}</span>
                    <span>${(p * 100).toFixed(1)}%</span>
                  </div>
                  <div class="prob-bar-track" style="height: 6px;">
                    <div class="prob-bar-fill ${idx === 0 ? 'winner' : ''}" style="width: ${Math.max(p * 100, 2)}%;"></div>
                  </div>
                </div>
              `;
            }).join('')}
          </div>
        </div>
      `;
    }

    this.resultsContainer.innerHTML = html;
  }

  renderDualResults() {
    const vej = this.results.vej;
    const jev = this.results.jev;

    let bannerHtml = '';
    if (vej.latency_ms && jev.latency_ms) {
      const vejLat = vej.latency_ms;
      const jevLat = jev.latency_ms;
      if (vejLat < jevLat) {
        const ratio = (jevLat / vejLat).toFixed(1);
        bannerHtml = `
          <div style="background: rgba(16, 185, 129, 0.1); border: 1px solid rgba(16, 185, 129, 0.3); border-radius: 8px; padding: 0.65rem 1rem; font-size: 0.82rem; font-weight: 600; color: #34d399; margin-bottom: 0.75rem;">
            ⚡ Benchmark Result: local <b>vej</b> completed in <b>${vejLat.toFixed(1)}ms</b> vs hosted <b>Jev</b> in <b>${jevLat.toFixed(1)}ms</b> (${ratio}x faster locally)!
          </div>
        `;
      } else if (vejLat > jevLat) {
        const ratio = (vejLat / jevLat).toFixed(1);
        bannerHtml = `
          <div style="background: rgba(168, 85, 247, 0.1); border: 1px solid rgba(168, 85, 247, 0.3); border-radius: 8px; padding: 0.65rem 1rem; font-size: 0.82rem; font-weight: 600; color: #c084fc; margin-bottom: 0.75rem;">
            ☁️ Benchmark Result: hosted <b>Jev</b> completed in <b>${jevLat.toFixed(1)}ms</b> vs local <b>vej</b> in <b>${vejLat.toFixed(1)}ms</b> (hosted Jev was ${ratio}x faster)
          </div>
        `;
      } else {
        bannerHtml = `
          <div style="background: rgba(59, 130, 246, 0.1); border: 1px solid rgba(59, 130, 246, 0.3); border-radius: 8px; padding: 0.65rem 1rem; font-size: 0.82rem; font-weight: 600; color: #60a5fa; margin-bottom: 0.75rem;">
            ⚡ Benchmark Result: local <b>vej</b> and hosted <b>Jev</b> completed with equal latency (<b>${vejLat.toFixed(1)}ms</b>)!
          </div>
        `;
      }
    } else {
      bannerHtml = `
        <div style="background: rgba(100, 116, 139, 0.1); border: 1px solid rgba(100, 116, 139, 0.3); border-radius: 8px; padding: 0.65rem 1rem; font-size: 0.82rem; font-weight: 600; color: #94a3b8; margin-bottom: 0.75rem;">
          Benchmark Comparison: local <b>vej</b> (${vej.latency_ms ? vej.latency_ms.toFixed(1) + 'ms' : 'ERR'}) vs hosted <b>Jev</b> (${jev.latency_ms ? jev.latency_ms.toFixed(1) + 'ms' : 'ERR'})
        </div>
      `;
    }

    const vejAnswers = vej.data?.answers || {};
    const jevAnswers = jev.data?.answers || {};
    const allKeys = new Set([
      ...Object.keys(this.questions || {}),
      ...Object.keys(vejAnswers),
      ...Object.keys(jevAnswers)
    ]);
    const qNames = Array.from(allKeys);

    let html = `
      ${bannerHtml}
      <div style="display: flex; gap: 0.75rem; margin-bottom: 0.75rem;">
        <div style="flex: 1; padding: 0.6rem 0.8rem; background: var(--bg-surface); border: 1px solid var(--border-color); border-radius: 8px; font-size: 0.75rem;">
          <div style="font-weight: 700; color: #60a5fa; margin-bottom: 0.2rem;">⚡ Local vej</div>
          <div style="color: var(--text-muted);">Latency: <b>${vej.latency_ms ? vej.latency_ms.toFixed(1) + ' ms' : 'ERR'}</b> | Input: <b>${esc(vej.data?.usage?.input_tokens ?? vej.data?.tokens_evaluated ?? 0)}</b> | Output: <b>0 tok (pure softmax)</b></div>
          ${vej.error ? `<div style="color: #fb7185; margin-top: 4px;">${esc(vej.error)}</div>` : ''}
        </div>
        <div style="flex: 1; padding: 0.6rem 0.8rem; background: var(--bg-surface); border: 1px solid var(--border-color); border-radius: 8px; font-size: 0.75rem;">
          <div style="font-weight: 700; color: #a855f7; margin-bottom: 0.2rem;">☁️ Hosted Jev</div>
          <div style="color: var(--text-muted);">Latency: <b>${jev.latency_ms ? jev.latency_ms.toFixed(1) + ' ms' : (jev.error ? 'FAILED' : 'ERR')}</b> | Input: <b>${esc(jev.data?.usage?.input_tokens ?? 0)}</b> | Output: <b>${esc(jev.data?.usage?.output_tokens ?? 0)} tok</b></div>
          ${jev.error ? `<div style="color: #fb7185; margin-top: 4px;">${esc(jev.error)}</div>` : ''}
        </div>
      </div>
    `;

    for (const qName of qNames) {
      const qSpec = this.questions[qName] || {};
      const vejAns = this.normalizeAnswer(vejAnswers[qName], vej.data);
      const jevAns = this.normalizeAnswer(jevAnswers[qName], jev.data);

      const vejScoreSub = (vejAns?.type === 'score') ? this.getScoreLabelSub(qName, vejAns.score) : '';
      const jevScoreSub = (jevAns?.type === 'score') ? this.getScoreLabelSub(qName, jevAns.score) : '';

      html += this.renderQuestionHeader(qName, qSpec);
      html += `
        <div class="compare-container" style="margin-bottom: 0.75rem;">
          <!-- vej Card -->
          <div class="compare-card winner-card">
            <div style="display: flex; justify-content: space-between; align-items: center;">
              <span style="font-weight: 800; font-size: 0.88rem; color: #60a5fa;">⚡ Local vej</span>
              <span class="metric-pill" style="color: #34d399; font-weight: 700;">${((vejAns?.confidence || 0) * 100).toFixed(1)}% conf</span>
            </div>
            <div>
              <div style="font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase;">Verdict</div>
              <div style="font-size: 1.25rem; font-weight: 800; color: #f8fafc;">${esc(vejAns?.choice || (vej.error ? 'ERR' : 'N/A'))}</div>
              ${vejScoreSub ? `<div style="font-size: 0.78rem; font-weight: 600; color: #93c5fd; margin-top: 2px;">${esc(vejScoreSub)}</div>` : ''}
            </div>
            <div class="dist-card" style="padding: 0.5rem; background: var(--bg-surface);">
              ${Object.entries(vejAns?.probabilities || {}).sort((a,b)=>b[1]-a[1]).map(([k, p], idx) => {
                const optLabel = this.getOptionLabel(qName, k);
                return `
                  <div class="prob-row">
                    <div class="prob-meta" style="font-size: 0.75rem;">
                      <span>${idx === 0 ? '🏆 ' : ''}${esc(optLabel)}</span>
                      <span>${(p * 100).toFixed(1)}%</span>
                    </div>
                    <div class="prob-bar-track" style="height: 6px;">
                      <div class="prob-bar-fill ${idx === 0 ? 'winner' : ''}" style="width: ${Math.max(p * 100, 2)}%;"></div>
                    </div>
                  </div>
                `;
              }).join('')}
            </div>
          </div>

          <!-- Jev Card -->
          <div class="compare-card">
            <div style="display: flex; justify-content: space-between; align-items: center;">
              <span style="font-weight: 800; font-size: 0.88rem; color: #a855f7;">☁️ Hosted Jev</span>
              <span class="metric-pill" style="${jev.error ? 'color: #fb7185;' : ''}">${jevAns?.confidence !== undefined ? ((jevAns.confidence) * 100).toFixed(1) + '% conf' : 'N/A'}</span>
            </div>
            <div>
              <div style="font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase;">Verdict</div>
              <div style="font-size: 1.25rem; font-weight: 800; color: ${jev.error ? '#fb7185' : '#f8fafc'};">${esc(jevAns?.choice || (jev.error ? 'Error' : 'N/A'))}</div>
              ${jevScoreSub ? `<div style="font-size: 0.78rem; font-weight: 600; color: #d8b4fe; margin-top: 2px;">${esc(jevScoreSub)}</div>` : ''}
            </div>
            ${!jev.error ? `
            <div class="dist-card" style="padding: 0.5rem; background: var(--bg-surface);">
              ${Object.entries(jevAns?.probabilities || {}).sort((a,b)=>b[1]-a[1]).map(([k, p], idx) => {
                const optLabel = this.getOptionLabel(qName, k);
                return `
                  <div class="prob-row">
                    <div class="prob-meta" style="font-size: 0.75rem;">
                      <span>${idx === 0 ? '🏆 ' : ''}${esc(optLabel)}</span>
                      <span>${(p * 100).toFixed(1)}%</span>
                    </div>
                    <div class="prob-bar-track" style="height: 6px;">
                      <div class="prob-bar-fill" style="width: ${Math.max(p * 100, 2)}%;"></div>
                    </div>
                  </div>
                `;
              }).join('')}
            </div>
            ` : ''}
          </div>
        </div>
      `;
    }

    this.resultsContainer.innerHTML = html;
  }
  renderCodeExport() {
    const payload = this.buildPayload();
    const payloadJson = JSON.stringify(payload, null, 2);
    const host = this.target === 'jev' ? 'https://api.typesafe.ai' : 'http://127.0.0.1:8080';
    const authHeader = this.target === 'jev' 
      ? `-H "Authorization: Bearer $TYPESAFE_API_KEY" \\\n  `
      : '';

    let code = '';

    if (this.codeLang === 'curl') {
      code = `curl -X POST ${host}/v1/systemone \\
  ${authHeader}-H "Content-Type: application/json" \\
  -d '${payloadJson}'`;
    } else if (this.codeLang === 'python') {
      code = `${this.target === 'jev' ? 'import os\n' : ''}import requests

url = "${host}/v1/systemone"
headers = {
    "Content-Type": "application/json",
    ${this.target === 'jev' ? `"Authorization": f"Bearer {os.environ['TYPESAFE_API_KEY']}"` : ''}
}
payload = ${JSON.stringify(payload, null, 4)}

response = requests.post(url, headers=headers, json=payload)
print(response.json())`;
    } else if (this.codeLang === 'ts') {
      code = `const response = await fetch("${host}/v1/systemone", {
  method: "POST",
  headers: {
    "Content-Type": "application/json",
    ${this.target === 'jev' ? '"Authorization": `Bearer ${process.env.TYPESAFE_API_KEY}`' : ''}
  },
  body: JSON.stringify(${payloadJson})
});

const result = await response.json();
console.log(result);`;
    } else if (this.codeLang === 'rust') {
      code = `use reqwest::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    let res = client.post("${host}/v1/systemone")
        .header("Content-Type", "application/json")
        ${this.target === 'jev' ? '.header("Authorization", format!("Bearer {}", std::env::var("TYPESAFE_API_KEY")?))' : ''}
        .body(r#"${payloadJson}"#)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    println!("{:#?}", res);
    Ok(())
}`;
    }

    const codeEl = document.getElementById('code-output');
    if (codeEl) {
      codeEl.textContent = code;
    }
  }

  showToast(msg, isError = false) {
    this.toastEl.textContent = msg;
    this.toastEl.style.borderColor = isError ? '#f43f5e' : '#10b981';
    this.toastEl.classList.add('show');
    setTimeout(() => {
      this.toastEl.classList.remove('show');
    }, 2500);
  }
}

// Initialize on DOM load
window.addEventListener('DOMContentLoaded', () => {
  new VejPlayground();
});

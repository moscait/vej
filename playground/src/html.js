// HTML escaping for values interpolated into innerHTML templates.
// Every user- or server-supplied value (question names, criteria, instructions, model ids,
// verdicts, error messages, responses relayed from TypeSafe) must pass through escapeHtml.

const HTML_ESCAPES = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
};

/** Escapes `& < > " '` so the value is safe in element content and quoted attributes. */
export function escapeHtml(value) {
  if (value === null || value === undefined) return '';
  return String(value).replace(/[&<>"']/g, (ch) => HTML_ESCAPES[ch]);
}

/** Catalog ids used when the daemon's /v1/models cannot be reached. Mirrors RECOMMENDED_MODELS. */
export const FALLBACK_MODEL_IDS = [
  'llama-3.2-1b',
  'smollm2-1.7b',
  'llama-3.2-3b',
  'llama-3.1-8b',
  'granite-4.2-3b',
  'gemma-4-e2b',
];

/**
 * Turns a /v1/models response into select options.
 * Returns [{ value, label, disabled, active }]; falls back to the catalog ids when the
 * response is missing or malformed.
 */
export function modelOptionsFrom(models) {
  if (!Array.isArray(models) || models.length === 0) {
    return FALLBACK_MODEL_IDS.map((id) => ({ value: id, label: id, disabled: false, active: false }));
  }
  return models
    .filter((m) => m && typeof m.id === 'string' && m.id.length > 0)
    .map((m) => {
      const isDownloaded = m.is_downloaded !== false;
      const isActive = m.is_active === true;
      let label = m.id;
      if (isActive) label += ' (loaded)';
      if (!isDownloaded) label += ' (not downloaded)';
      return { value: m.id, label, disabled: !isDownloaded, active: isActive };
    });
}

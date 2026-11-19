-- Phase 5 M20: the multi-provider model router. `model_configs.provider`
-- has existed since the M1 schema but every row seeded so far has been
-- `'anthropic'` — this adds one real, currently-valid model id per new
-- provider (`agent::provider` now has a real `ModelProvider` impl for each)
-- so the Settings page's model-config list and the new per-role
-- (`model.role.*`) assignment dropdowns have something genuine to offer
-- beyond Anthropic. None of these is `is_default` — the existing
-- `default-sonnet` row seeded by `0001_init.sql` stays the one and only
-- global default, so an existing install's behavior is completely
-- unaffected by this migration; these rows only become reachable once a
-- user explicitly picks one (as a role default, or later, directly).
--
-- Model ids, and why each was picked as a real, non-placeholder identifier:
--   * OpenAI:     'gpt-4.1'                             — a real, documented OpenAI Chat Completions model id.
--   * Google:     'gemini-2.5-pro'                       — a real, documented Gemini model id (used as the `{model}` path segment in `generateContent`/`streamGenerateContent`).
--   * OpenRouter: 'meta-llama/llama-3.1-70b-instruct'    — OpenRouter's own `<publisher>/<model>` id convention; a real, previously-listed OpenRouter model id, though OpenRouter's catalog changes over time so this is worth confirming against OpenRouter's live model list before relying on it.
INSERT INTO model_configs (id, provider, model_id, display_name, is_default, max_output_tokens)
VALUES
  ('default-gpt-4-1', 'openai', 'gpt-4.1', 'GPT-4.1', 0, 16384),
  ('default-gemini-2-5-pro', 'google', 'gemini-2.5-pro', 'Gemini 2.5 Pro', 0, 8192),
  ('default-openrouter-llama-3-1-70b', 'openrouter', 'meta-llama/llama-3.1-70b-instruct', 'OpenRouter: Llama 3.1 70B Instruct', 0, 8192);

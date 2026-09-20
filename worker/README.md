# Hosted preview worker

This directory contains only the optional static hosted-preview adapter.

- `index.js` serves built assets and applies a single-page application fallback for navigation requests.
- `hosting.json` is the source manifest copied to `dist/.openai/hosting.json` by `scripts/prepare-sites-build.mjs`.

Neither file is loaded by the Tauri desktop application or `ct-engine`, and this module must never expose device-control endpoints. Standard JSON does not support comments, so the manifest is documented here instead of adding invalid inline syntax.

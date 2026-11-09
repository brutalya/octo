# Stage 6 — image and PDF attachments

Goal: a photo or a PDF (e.g. lab results) uploaded on the phone into the private space is processed by a local model on the PC. Audio (speech-to-text) is later.

## Kinds (by content sniffing of the first bytes, not by extension)
- Text: unchanged (utf-8, ≤ 200 KB, inlined).
- Image: PNG, JPEG, WebP, GIF; ≤ 10 MB.
- PDF: text extracted in memory (pure-Rust crate, no external tools), result treated as a text attachment (cap 200 KB, truncated with a marker line). No text layer (scanned PDF) -> 415.
- Anything else -> 415 (unchanged).
- Private files are decrypted in memory only (unchanged rule).

## Providers
- Ollama: images go as base64 in `images` of the user message carrying them. Before sending, the model's capabilities are checked once via `POST /api/show` (`capabilities` contains `vision`), cached per model for the process lifetime; no vision -> 415 with nothing stored.
- OpenRouter (public only): content parts `[{type:text},{type:image_url,image_url:{url:"data:<mime>;base64,..."}}]`.
- CLI providers (public only): images are not inlined; the prompt gets one line per image `Attached image: <absolute path in the public dir>` (both CLIs can read files in their working dir / `--add-dir`).
- History: images are sent only with the turn that carries them (same token rule as text attachments).

## API / UI
- No new endpoints. Attachment refs unchanged; the server decides the kind.
- Files panel: image preview (fetched as blob -> object URL, no remote URL). PDF: no preview.
- Chat: attachment chips unchanged (basename).

## Tests first
Sniffing (png/jpeg/webp/gif/pdf/text/binary), size caps, PDF extraction on a small generated fixture (with and without text layer), Ollama request body with images (fake server) incl. `/api/show` capability check and cache, OpenRouter content parts, CLI prompt line, private image to Ollama end to end via the API, 415 cases (non-vision model, unknown binary, scanned PDF).

## Deviations
- PDF text comes from `pdf-extract` (built on `lopdf`; handles CID/Type0 fonts with ToUnicode CMaps, e.g. Cyrillic). Input is capped at 20 MB (415 above), parsing runs inside `spawn_blocking` together with the file read, and the parser is wrapped in `catch_unwind` because it panics on some malformed files; any parse failure is 415. The 200 KB cap includes the `[truncated]` marker line.
- Sniffing: text is "no NUL byte and valid UTF-8", so an empty file is text. A non-vision or unknown-capability check failure of `/api/show` (unreachable, non-2xx) is an upstream error (502), and is not cached. The vision cache key is (`ollama_url`, model).
- Images ride on the last message only, passed to `stream_chat` as a separate `images` argument (`ChatMessage` is unchanged). For CLI providers images are referenced by absolute public path and never inlined; the vision check applies to Ollama only.
- Web: image preview is chosen by file extension (png, jpg, jpeg, webp, gif); the object URL is revoked when the preview is closed or replaced.

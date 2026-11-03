// Hand-written until the Rust side regenerates these via ts-rs (docs/stage1.md).
export type ModelInfo = { id: string, name: string, };
export type FileKind = "file" | "dir";
export type FileEntry = { path: string, kind: FileKind, size: number, modified: number, };
export type Attachment = { space: "private" | "public", path: string, };
export type Settings = { ollama_url: string, openrouter_api_key: string | null, history_char_budget: number, };
export type SettingsPatch = { ollama_url?: string, openrouter_api_key?: string, history_char_budget?: number, };

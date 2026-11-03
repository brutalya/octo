// Hand-written to match docs/prototype.md. Replaced by ts-rs output later.
export type Space = 'private' | 'public';
export type ProviderKind = 'claude' | 'antigravity';
export type Role = 'user' | 'assistant' | 'error';

export interface Thread {
  id: string;
  title: string;
  provider: ProviderKind;
  space: Space;
  created_at: number;
  updated_at: number;
}

export interface Message {
  id: string;
  thread_id: string;
  role: Role;
  content: string;
  created_at: number;
}

export interface ProviderStatus {
  kind: ProviderKind;
  installed: boolean;
  version: string | null;
  is_local: boolean;
}

export type AgentEvent =
  | { type: 'session'; id: string }
  | { type: 'text_delta'; text: string }
  | { type: 'tool'; name: string; summary: string }
  | { type: 'done' }
  | { type: 'error'; message: string };

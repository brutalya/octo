# Stage 4b — private files sync

Goal: the private space is the same on every full-trust node (PC, phone). Public-trust nodes (e.g. a VPS) never receive private data, not even ciphertext. Thread sync is 4c.

## Step 1 — core: versioned private store (octo-core)
- Index entry gains `rev: {clock: u64, node: String}`, `hash: String` (BLAKE2s-256 hex of plaintext, files only), `deleted: bool` (tombstone). Old indexes load with `rev {0, ""}`, hash computed on open, `deleted false`; saved back once.
- `PrivateFiles::open(root, key, node_id)`. Lamport clock: every local change (write, mkdir, rename, delete) sets `rev = {max_clock_seen + 1, node_id}`. Rename = tombstone at old path + new entry (same blob, same hash). Delete = tombstone (blob removed, entry kept). Recreating a tombstoned path is a normal write with a new rev.
- `list`, `read`, `write` etc. ignore tombstones (existing API behaviour unchanged).
- `manifest() -> Vec<SyncEntry>` incl. tombstones; `SyncEntry {path, kind, size, modified, hash, rev, deleted}` (serde, ts-rs not needed).
- Pure `plan(local: &[SyncEntry], remote: &[SyncEntry]) -> Vec<Action>`: per path, remote wins iff `remote.rev > local.rev` (compare clock, then node string); `Action::Fetch(entry)` when the winner is a live file whose hash is not present locally under any path, `Action::Adopt(entry)` otherwise (dir, tombstone, or file with a local blob of the same hash). Missing locally = loses to anything.
- `apply(entry, data: Option<&[u8]>)`: verifies `hash(data) == entry.hash` (else error, nothing changed), stores with the remote rev unchanged, bumps local `max_clock_seen`. Adopting a file over a non-empty dir or under a file parent is skipped (error returned, caller logs).
- `read_by_hash(hash) -> Option<Vec<u8>>`.
- Tests first: merge rules (newer wins, tie by node, tombstone wins over older file, older remote ignored, idempotent), rename/delete produce tombstones, migration of a v1 index, hash mismatch rejected, Fetch vs Adopt by local hash.

## Step 2 — server: pull sync between full members
- Routes (local router, reachable through the tunnel): `GET /api/sync/private/manifest`, `GET /api/sync/private/blob/:hash`. Tunnel policy: allowed only for `full` members; `public` members get 403. Local callers may use them too (harmless).
- Worker in the node actor, per full member, at most one run per peer at a time: on session established, on a `{"t":"sync_hint"}` IK message, and every 60 s while online. Run = GET manifest → `plan` → Fetch blobs (each verified) → `apply` → done. Pull only; the peer pulls from us symmetrically.
- After a local private change the node sends `sync_hint` to online full members (debounced 1 s). Hints from non-full members are ignored.
- Private key catch-up: on session established with a full member, both send `{"t":"private_epoch",epoch}`; the side with the newer epoch sends `private_key` to the other (existing rules for adopting apply).
- Tests first: two nodes + relay in one process (existing harness): A writes a private file → B (full) has it; B deletes → A loses it; concurrent edits → higher rev wins on both; C (public) gets 403 on both routes and never receives files; offline B catches up after reconnect; catch-up of a rotated key.

## Step 3 — relay mailbox (later)
Store-and-forward for full members that are never online at the same time. Decided when step 2 is in use; the relay would only ever hold ciphertext under keys it does not have.

## Decisions
- LWW per path by Lamport rev; the losing concurrent edit is lost (no conflict copies yet).
- Transfer runs over the existing IK sessions (end-to-end encrypted node to node). Private data is sent only to nodes this node marks `full`.
- Tombstones are kept forever (small); GC later.

## Deviations (Step 1)
- `Hub::new(config, node_id)` and `Files::open(data_dir, node_id)` take the node id; the server and desktop pass `octo_server::node::node_id(data_dir)` (derived from `keys/node.key`, the same key the Node uses).
- New `FilesError::HashMismatch` (HTTP 400) for a failed hash check in `apply`.
- `apply` is a no-op (Ok) when the local entry's rev is not older than the remote's; missing parent directories created by `apply` get rev `{0, ""}`.

## Deviations (Step 2)
- Each node keeps its own at-rest files key (`keys/private.key`); the membership `private_key`/epoch is separate. Sync moves plaintext over the IK sessions, and epoch catch-up concerns only the membership key.
- Sync rounds also start when the relay link becomes ready and when a session with a full member is established (both sides); the 60 s run is a fallback for established peers.
- `private_epoch` is sent on session establishment to full members; the side with the newer epoch replies with `private_key`.
- Local private changes are detected in the HTTP layer (successful non-GET on `/api/files/private`, also via the tunnel); remote applies do not hint.
- Blobs and the manifest are size-capped (50 MB / 32 MB); sync tasks run off the node actor.
- `apply` and `plan` reject remote entries with a non-normalized or root path, a malformed live-file hash, or data given for a directory or tombstone.
- A failed sync run (session churn) is retried with backoff; dropping a mux fails pending stream reads at once; a failed probe resets a session only if it is still the one the probe used.
- Audit fixes: `plan` leaves out entries `apply` would reject forever (a remote tombstone or file over a directory that keeps live children; an entry below a live file) and orders tombstones last, deepest first, so repeated rounds converge. `apply` still returns an error for those if called directly. A tombstoned parent of an applied entry is revived with a revision newer than the tombstone so the revival propagates. A file-vs-non-empty-directory conflict is left as is on each side until one changes.
- `PrivateFiles::apply_batch` / `Files::private_apply_batch` apply many entries with one index save. Live files whose legacy blob cannot be read during hash migration are left out of the manifest and listed by `unreadable()`. `open` removes stale `index.bin.tmp` and unreferenced blobs. The index stays a `Vec` (every operation re-encrypts the whole index anyway).
- Paths with `:` or a Windows device name (`CON`, `NUL`, `COM1`, ... also with an extension) are rejected; the public listing skips non-UTF-8 names. `private.key` is created atomically (temp file + hard link) and zeroized on drop.


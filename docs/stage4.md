# Stage 4 — nodes, E2EE tunnel, remote control

Every installation (PC, VPS, later phone) is a **node**. A **relay** connects nodes that can't reach each other. The relay never sees plaintext and holds no keys. A user's own VPS runs both a node and a relay; a public relay for other people is the same binary with only the relay role.

## Identity & pairing
- Node identity: static X25519 keypair (Noise) + node id = first 16 bytes of BLAKE2s(pubkey), base32. Stored in `<data>/keys/node.key` (0600). Created on first start.
- A **network** = set of paired nodes of one owner. Membership list stored on each node: `{node_id, pubkey, name, added_at}`.
- Pairing (new node B joins A): A shows a one-time code `relay_url#token` (QR later) valid 5 min. B connects to the relay, sends join with the token; A and B run Noise `XX`, then both show a 6-digit SAS derived from the handshake hash; the user confirms on A. After confirmation A sends B the membership list and the **network key** (32 bytes, used for stage 4b sync), and A broadcasts B's pubkey to other members.
- Unpair: remove from list on all nodes; rotate network key (4b).

## Transport
- Relay: WebSocket server (`octo-server --relay`, path `/relay`). Clients authenticate to the relay by signing a relay challenge with their static key (Noise `IK`-style proof or Ed25519 derived — choose the simpler correct one, document it). Relay routes binary frames `{to: node_id, payload}` only between nodes that both registered the same `network_id` (hash of network key's public commitment) — no broadcast to strangers, rate limits per connection.
- End-to-end: each node pair runs Noise `IK` (`snow` crate, `Noise_IK_25519_ChaChaPoly_BLAKE2s`) over relay frames. Rekey per session; replay protection by Noise nonces. Direct LAN/WAN connection is a later optimization (same Noise, different carrier).
- Multiplexing inside a Noise session: stream frames `{stream_id, kind: open|data|end|reset, bytes}`; max frame 64 KB; backpressure via per-stream window.

## Remote control (the 4a deliverable)
- Remote API = the same HTTP API, tunneled: node A's server exposes `/api/nodes` (list members + online status) and `/api/nodes/:node_id/*` which forwards the request over the tunnel to node B's local router (in-process, not via TCP) and streams the response back (SSE works unchanged).
- Authorization: requests arriving through the tunnel are accepted only from member nodes. Policy unchanged and enforced on the target node: private threads/files of B are only usable by B's local providers; tunneled requests may read B's private data only if the requesting node is a member (it is E2EE end to end).
- Web: node switcher at the bottom of the sidebar (local + members, online dot). Switching re-targets the API base to `/api/nodes/:id`. Pairing UI in settings: "Add device" shows code; "Join" takes a code; SAS confirm dialog.

## 4b (next, not now)
Encrypted store-and-forward sync of private files and thread metadata via relay mailbox, encrypted with the network key (XChaCha20-Poly1305), conflict policy last-writer-wins per file version.

## Tests (first)
Noise handshake + transport roundtrip between two in-memory nodes; relay routing isolation (node of another network can't receive frames); tampered frame rejected; replay rejected; SAS equal on both sides and differs under MITM (relay swapping keys); stream mux with interleaved streams + backpressure; tunneled SSE end-to-end with two nodes + relay in one test process; non-member rejected.

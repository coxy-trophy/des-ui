# Explicit TLS file transfers

`src/transfers.rs` implements independent file-payload jobs for a currently paired
DES Connect control session. It does not implement discovery, pairing, a control
listener, URL opening, clipboard text, remote filesystem browsing, or automatic
file opening. It creates a transient payload listener only after an explicit local
send action. Incoming metadata alone starts no connection, creates no payload
file, and starts no worker thread; the user must accept it locally.

## Wire behavior verified against the Android source

The checked-in Android implementation is the source of these interoperability
choices, rather than assumptions about the main control channel's TLS roles:

- `LanLink.sendPacket` binds a payload port, adds
  `payloadTransferInfo: {"port": PORT}`, and sends the control packet first.
- `LanLink.sendPayload` accepts the payload TCP connection and calls
  `convertToSslSocket(..., isDeviceTrusted=true, clientMode=false)`: **the file
  sender is the payload TLS server**.
- `LanLink.receivedNetworkPacket` connects to the main control socket's actual
  remote address, then uses `clientMode=true`: **the receiver is the TLS client**.
- `LanLinkProvider` allocates payload ports in 1739–1764. `SslHelper` uses TLS 1.2
  and requires a client certificate for a trusted server-side payload socket.
- `SharePlugin` identifies a file by `body.filename`. Its batch metadata uses
  `numberOfFiles` and `totalPayloadSize`. `open` can request automatic opening in
  Android; DES sends `open:false` and ignores incoming opening requests.

The outbound packet is:

```json
{
  "id": 0,
  "type": "kdeconnect.share.request",
  "body": {
    "filename": "photo.jpg",
    "numberOfFiles": 1,
    "totalPayloadSize": 12345,
    "open": false
  },
  "payloadSize": 12345,
  "payloadTransferInfo": {"port": 1739}
}
```

See the local Android sources:
[LanLink](../../des-connect-android/src/main/java/org/kde/kdeconnect/backends/lan/LanLink.kt),
[LanLinkProvider](../../des-connect-android/src/main/java/org/kde/kdeconnect/backends/lan/LanLinkProvider.kt),
[SslHelper](../../des-connect-android/src/main/java/org/kde/kdeconnect/helpers/security/SslHelper.kt),
and [SharePlugin](../../des-connect-android/src/main/java/org/kde/kdeconnect/plugins/share/SharePlugin.kt).
These were inspected at local revision `ae03898121968fecb5466f59eefc5c76638fa4c3`.
The upstream [protocol example](https://github.com/KDE/kdeconnect-meta/blob/work/protocol-schemas/protocol.md)
also describes the top-level payload fields. The source takes precedence over a
schema example that inconsistently abbreviates `payloadTransferInfo`.

Unmodified Android currently waits only **10 seconds** for the receiver to connect.
DES keeps a local incoming offer for 30 seconds; accepting an older Android offer
can therefore fail because its sender has already closed the listener. Ask that
sender to retry. A longer sender wait in the DES Android fork can improve this UX;
DES does not connect before consent to keep an upstream offer alive.

## Root service integration

Enable the existing `rustix` dependency's `fs` feature alongside `net` and `process`.
No additional crate is required. Add `pub mod transfers` in the library.

Construct `PeerTls` only inside an **authorized, paired control actor**:

```rust
let peer = PeerTls::new(
    local_key.clone(), local_certificate.clone(), peer_id.clone(),
    authenticated_peer_certificate.clone(), stored_paired_pin,
    authenticated_control_socket.peer_addr()?.ip(),
)?;
let mut transfers = TransferManager::new(default_download_directory()?)?;
```

The constructor checks the peer's self-signed certificate, device-ID/CN binding,
validity, and exact SHA-256 certificate pin; it also checks the local key/certificate.
The caller must obtain the pin from validated paired state. A digest merely
computed from an unpaired certificate is not authorization. A peer IP or path
provided in a remote packet must never be substituted for these local inputs.

The API returns `Result<T, String>` unless stated otherwise:

- `offer_receive(peer: PeerTls, packet: &Packet) -> TransferSnapshot` validates a
  file header and registers an unaccepted offer. Call it only after authorizing
  the control session; do not pass unpaired share packets into the manager.
- `accept(id: &str) -> TransferSnapshot` starts receiving after **explicit local
  approval**, while the matching peer is still paired and connected. IDs are
  cryptographically random 128-bit values checked for collision; stale approvals
  cannot target a new job after a manager restart.
- `send(peer: PeerTls, user_selected_path: &Path) -> SendTicket` opens only the
  locally selected regular file and binds the listener before returning
  `SendTicket { id, packet }`. Immediately send `packet` through that same peer's
  authenticated control actor. If sending the control header fails, call
  `cancel(&id)` so the listener closes promptly.
- `snapshots() -> Vec<TransferSnapshot>` returns newest-first, coalesced progress.
- `cancel(id: &str)` cancels an offered or active job. Repeating it for a retained
  terminal job succeeds without deleting completed files.
- `cancel_peer(peer_id: &str)` cancels all of that peer's offers and jobs. Call it
  on control disconnect, unpair, pin change, and service shutdown. The module
  intentionally does not treat a cached `PeerTls` as ongoing pairing authority.
- Dropping the manager cancels and shuts down its owned payload sockets, then
  joins its owned workers. No shared service or unrelated process is stopped.

Snapshots serialize `id`, `peer_id`, `direction` (`send`/`receive`), `filename`,
`size`, `transferred`, `state`, `error`, and `path`. The receive path is populated
only after successful finalization. States are `offered`, `connecting`, `waiting`,
`transferring`, `complete`, `failed`, `cancelled`, and `expired`. A sender's
`complete` means its bytes were written to the authenticated payload socket;
this protocol provides no separate durable-receipt acknowledgment from the peer.

## Bounds and lifecycle

- At most **two worker threads**, **eight unaccepted incoming offers**, and
  **32 total job/history records** per manager. Terminal history is evicted first;
  accepting or sending beyond the active limit fails. Poll snapshots even when
  the transfer panel is closed so expired offers release their retained metadata.
- Each file must have a known size of **1 byte–1 GiB**. Indefinite streams and empty
  files are refused; Android's `NetworkPacket.hasPayload()` excludes zero-length
  payloads. Multi-file batches are handled as independent, individually approved
  files; directories are not sent.
- File bytes stream through a **64 KiB buffer**. Progress overwrites a small shared
  snapshot rather than entering a queue; neither file bytes nor per-chunk logs are
  accumulated in memory.
- Outbound listener/valid-peer wait: 30 seconds, including wrong-certificate
  attempts; at most eight failed TLS attempts. Source IP must match the control
  peer before a handshake. Each TLS handshake has an absolute five-second
  deadline, independent of how often bytes arrive. The payload listener closes
  as soon as its legitimate peer authenticates.
- Incoming TCP connect: at most three seconds. Byte transfer: ten-second stall
  timeout and 30-minute maximum byte phase. TLS and payload loops are nonblocking
  and check cancellation regularly. Cancellation shuts down the actual registered
  TCP socket; it is not just a UI flag. A connect already in progress may take up
  to its three-second connect timeout to return.
- EOF before the authenticated advertised size fails and removes the partial.
  Bytes beyond that size are never written. Send sources are opened once; a
  source that shrinks fails, and a changed final length is reported. Editing a
  source in place while sending is not a supported snapshot workflow.
- Local filesystem reads, writes and `fsync` obey the operating system and the
  selected filesystem; network deadlines cannot preempt a kernel-stalled disk
  operation. Use a healthy local filesystem. No additional background I/O threads
  are created to hide a stalled filesystem.

## Filesystem protection

The normal destination is `$HOME/Downloads/DESConnect`, selected locally. Its
parents are walked with directory FDs and `NOFOLLOW`; `..`, symlinks, foreign-owned
parents and group/other-writable parents are refused. Root-owned sticky temporary
directories are allowed for disposable private roots. Missing components are
created as mode 0700; an existing final directory must already be owned by the
current user and private. The module does not silently relax permissions or follow
a Downloads symlink. Select a real private directory if the normal destination is
redirected through a symlink.

Only a single filename of at most 220 UTF-8 bytes is accepted from a peer. Path
separators, traversal names, control characters and bidi direction controls are
rejected. A remote filename never chooses a local directory. Received files start
as random, exclusive, no-follow mode-0600 `.part` files relative to the retained
directory FD. The complete file is committed with atomic `RENAME_NOREPLACE`;
existing regular files and symlinks are preserved. Name collisions produce
`1-name.ext`, `2-name.ext`, and so on, bounded to 1000 attempts.

Failed or cancelled unfinished jobs unlink their partial file using that same
anchored directory FD. A cancellation racing a successful final commit can report
completion and preserve the completed file. No executable permissions are added,
and no application is launched. A process crash can leave its private `.part`
file; the module does not scan/delete arbitrary existing files on the next boot.

Locally selected send paths must be absolute and regular, with no symlink in any
component. The source is held by its opened FD, so later path replacement does not
redirect the upload. Nonblocking open prevents a selected FIFO from hanging before
its regular-file check. Send-parent permissions need not match the download
privacy policy, since the user explicitly selects the source and it is never
written by this module.

## Verification

Disposable local certificate fixtures exercise real mutual TLS end to end,
explicit approval before connection/data, collision preservation, and ignoring a
remote-provided host. An attacker presents the **same expected device CN/ID with a
different valid key and certificate**: it is refused, and the legitimate pinned
peer can still connect afterward. Other tests cover early EOF, removal of partials,
actual socket shutdown on cancellation, worker/offer/history limits, stale-offer
expiry, oversized sparse files, path traversal, symlinks, absolute TLS deadlines,
and listener release on cancellation/drop. No Android device, ADB command,
screen capture, desktop input, or existing user service is involved in these tests.

These tests establish library behavior, not physical Android file-transfer or the
root service's consent UI. Those must be verified separately after integration.

## Integrated daemon consent and policy

The daemon persists `files_enabled` per trusted peer, defaulting to false for
existing records. Remembering the same peer again preserves this permission.
Discovery and plaintext identities always advertise only the core capabilities.
Only the encrypted identity for an opted-in, pinned peer includes
`kdeconnect.share.request` in both directions. The actor retains the peer's secure
capabilities and uses the actual current control socket IP for payloads.

Private IPC operations are:

```text
set_files       {device_id, enabled: boolean}
reconnect       {device_id}
send_file       {device_id, path: absolute_local_file}
accept_transfer {device_id, transfer_id}
cancel_transfer {transfer_id, device_id?: optional_owner_check}
```

`set_files` requires an unlocked session and a trusted paired peer; offline trusted
peers can have their policy updated for their next connection. On an existing
connection, `Device.files_enabled` shows saved permission and
`Device.files_reconnect_needed` indicates that the current encrypted advertisement
no longer matches it. Android does not reliably reload plugins from an ordinary
in-session identity packet. The explicit `reconnect` action closes the authorized
actor's control socket without deleting its pin, cancels its transfers, waits for
the old route to close, then sends one discovery broadcast outside loopback test
mode. Its acknowledgment confirms the requested close, not successful reconnection.
There is no automatic reconnect loop or automatic approval after reconnection.

Sending and accepting require a live paired actor, enabled permission, a matching
current encrypted advertisement and an unlocked session. Sending additionally
requires the peer's secure incoming capabilities to include sharing. Incoming
headers require its secure outgoing share capability. Offers remain `offered`
until local acceptance. A disabled or locked incoming offer is refused without
starting a payload connection. `cancel_transfer` remains available while locked
or offline. Disconnect, address/capability-changing socket replacement, unpair, policy disable,
lock and daemon shutdown cancel affected unfinished transfers.

Content-start jobs retain a duplicate of the originating private IPC connection
and expire after four seconds in the actor queue. Nonblocking socket peeks reject
a disconnected requester. The actor rechecks these guards before manager start
and after preparation, cancelling a created send ticket if the requester expired,
disconnected or lost policy before its control header is sent. No queued request
is replayed automatically after a reconnect.

`state.transfers` contains the bounded newest-first snapshots described above.
Source preparation uses `PreparedSource::open` outside the global manager mutex,
then `send_prepared`; the compatibility `send` wrapper remains available to callers
that do not hold a shared lock. Lazy download-directory preparation also runs
outside that mutex, with a second check so racing preparation cannot replace an
existing manager. Existing actors bound concurrent preparations; no extra thread
is spawned per source-open request. These changes do not make kernel filesystem
I/O interruptible.

The watchdog revokes screens before inspecting transfers, uses `try_lock` for
transfer bookkeeping, and performs shell/peer checks outside the manager lock.
Shutdown stops screen capture before cancellation or worker joins. Thus a stuck
source filesystem cannot keep a screen viewer running behind a file-manager lock.
A deterministic isolated fixture holds that lock while an owned fake viewer runs;
a locked shell must still cause the viewer to stop promptly.

Daemon regression fixtures use temporary homes, private fake shell sockets,
disposable mutual TLS identities, and disposable file bytes. They verify the
permission/reconnect flow, secure per-peer capability gating, real send/receive
payloads, explicit approval, refusal of unknown peers, locked policy mutations,
and actual payload socket/partial cleanup on lock, unpair and orderly daemon stop.

The KDE Connect share namespace does not express separate file, URL and text
sub-capabilities. Advertising `kdeconnect.share.request` for files can therefore
also enable Android's external share target for text/URLs. This increment supports
file payloads only. A non-payload share packet publishes the device error
“URL/text sharing is not enabled; only file offers supported”; its private content
is neither retained nor opened. No URL/text feature is silently implied by the
file permission.


A routine discovery refresh can replace the control socket without disconnecting
its authenticated logical link. Existing payload jobs and pending local offers
survive that refresh only when the certificate digest is unchanged, the actual
peer IP is unchanged, the paired file permission is still enabled and unlocked,
the new encrypted identity advertises files, and the peer's secure incoming and
outgoing share-capability bits still match. Their existing IDs and consent deadlines
remain in force; a refresh neither accepts an offer nor restarts a cancelled job.
Address changes, share-capability changes, revoked policy and actual actor
termination still cancel affected jobs. Real TLS fixtures exercise both a preserved
pending offer that subsequently completes and cancellation on capability removal;
a separate unit guard covers an address change without changing a real interface.

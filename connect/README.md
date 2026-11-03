# DES Connect native foundation

This MIT crate is an independently implemented protocol/state foundation. It is
not a running service, TLS transport, discovery agent, UI integration or a tested
Android pairing implementation. No listener or remote command path is exposed.

`packet` reads one newline-delimited JSON packet at a time with a 64KiB bound,
including the newline, and bounds outgoing serialization. All read/validation
errors are connection-fatal: close the stream, never scan the leftover suffix for
a new packet. A future socket adapter also needs read deadlines and connection
limits. Payload metadata is preserved (including upstream's unknown-length -1),
but payload transport/file handling is not implemented here.

Identity accepts protocol version 8 only, validates device IDs, display names,
ports and capability-list bounds. Discovery identity and capability claims do not
establish trust. Schema and state behavior were studied from upstream Android
revision 3242aeff092bba165a64d63a690d31627064644d; no upstream implementation code
was copied into the MIT crate.

`pairing` models one peer per instance. Its opaque connection and attempt tokens
prevent disconnected or cancelled approvals/send callbacks from acting on a new
request. Incoming/outgoing deadlines are 25/30 monotonic seconds and incoming v8
clock tolerance is 1800 epoch seconds. UI rejection and asynchronous send callbacks
must carry the original attempt token. Acceptance is not authorization until its
send succeeds. Never initialize it with a digest copied from a discovery packet.

Before using the model with a network peer, the transport must implement mutual
TLS, key-possession validation, identity/certificate binding, persisted private
keys and certificate pins, and v8 public-key/timestamp verification-code display.
Restoring persisted trust is not yet implemented. The TLS adapter must attach
callbacks to the proper attempt and account for the protocol's response semantics;
this library cannot identify arbitrary delayed wire messages without transport
context. The model is not a substitute for encryption or a security audit.

Run `cargo test -p des-connect --locked` for bounded framing, identity validation,
trust transitions, cancellation, timeout, changed-key and stale-callback checks.

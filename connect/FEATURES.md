# Peer feature protocol module

`src/features.rs` is an independent Rust implementation of the packet contracts
observed in the local DES Connect Android fork. It creates packets and decodes
bounded data; it does not connect, execute commands, open URLs, touch the OS
clipboard, import contacts, write files, grant Android permissions or send packets.
It requires no new dependencies. Service/UI integration remains caller-owned.
No GPL implementation was copied into this MIT module.

## Integration contract

1. Authenticate the **current** paired transport/certificate and device identity
   before calling `decode` or `PeerState::receive`. Discovery identity and a device
   ID supplied by IPC are insufficient. Recheck current connection/trust before
   dispatching every outgoing operation.
2. Inspect the peer's advertised incoming/outgoing capability vectors.
   `PeerCapabilities::from_peer` supplies direction-aware feature hints, and
   `INCOMING_TYPES`/`OUTGOING_TYPES` enumerate this module's protocol vocabulary.
   For each outgoing packet, verify its exact type appears in the peer's incoming
   capabilities. Capability does not establish permission or user consent.
3. Apply explicit user opt-ins using `FeatureFlags` and `PeerState::set_enabled`.
   All flags default false, including omitted JSON fields. No subscribe or
   synchronization request is emitted on connection. In particular, clipboard
   synchronization is not automatic.
4. `PeerState::receive(&Packet)` returns a typed `Event` only for enabled incoming
   feature families, updating a bounded process-only event cache. `decode(&Packet)`
   is the stateless alternative; it does not enforce opt-ins or authentication.
   `events()` exposes oldest-to-newest cache entries. Call `clear()` on
   disconnect/unpair, lock or explicit Forget. `set_enabled` immediately drops
   entries belonging to newly disabled features.
5. Render text as inert plain text. Never log event bodies, clipboard contents,
   recipients, contact cards, notification reply IDs or cached private data.
   Error messages identify a field/limit, without embedding its private value.
   Require the unlocked owner for notification presentation and every OS write,
   open, reply, dismiss, send or transfer approval; enforce this again at dispatch.
6. Explicit actions must use current peer/card/message identities. A notification
   update or cancellation removes its previous cached notification, invalidating
   the old reply ID/action list. A reply must be tied to the current notification
   from this same peer; a dismiss must honor `clearable`. No constructor grants
   permission to invoke an arbitrary peer-supplied action automatically.

### Public API

- `decode(&Packet) -> Result<Event, FeatureError>`: URL/text offers, clipboard
  offers, notifications/cancellations, call alerts, SMS batches/attachment offers,
  contact timestamps and opaque inline vCards.
- `send_url`, `send_clipboard`, `find_phone`: explicit outbound requests.
- `request_notifications`, `dismiss_notification`, `reply_notification`,
  `notification_action`, `mute_ringer`: explicit notification/call controls.
- `request_sms_conversations`, `request_sms_thread`, `send_sms`,
  `request_sms_attachment`: explicit SMS/MMS read/send/transfer requests.
- `request_contacts_index`, `request_contact_cards`: explicit contact reads.
- `PeerState`: default-disabled, bounded ephemeral cache; no serialization to disk.

Every constructor returns a normal `Packet` with ID 0 and validated bounded
encoding. The existing transport owns its packet ID and paired send channel.
The caller retains drafts and performs confirmation before dispatch; building a
packet does not send it.

## Exact Android wire observations

Source root: `../des-connect-android/src/main/java/org/kde/kdeconnect/`.
The plugin namespace remains `org.kde.kdeconnect` despite DES Connect branding.

| Workflow | Sources inspected | Actual packet contract |
|---|---|---|
| URL/text | `plugins/share/SharePlugin.kt` | `kdeconnect.share.request`, `url` or `text`; `filename` takes precedence and belongs to the transfer module |
| Clipboard | `plugins/clipboard/ClipboardPlugin.kt` | `kdeconnect.clipboard` with `content`; `.clipboard.connect` has millisecond `timestamp` and optional `content` |
| Find phone | `plugins/findmyphone/FindMyPhonePlugin.kt` | Empty `kdeconnect.findmyphone.request`; receiving it can ring/show a phone UI |
| Notifications | `plugins/notifications/NotificationsPlugin.kt` | `.notification`: string `id`, optional/redacted `title`/`text`/`ticker`, `requestReplyId`, string `actions`, conversation `{sender,content}` entries; cancellation uses `isCancel:true` |
| Notification controls | Same | `.notification.request` with `request:true` or string `cancel`; `.notification.reply` with `requestReplyId` and `message`; `.notification.action` with `key` and `action` |
| Call alerts | `plugins/telephony/TelephonyPlugin.kt` | `.telephony` `event`: `ringing`, `talking`, `missedCall`; `phoneNumber`/`contactName` optional; actual cancellation emits string `isCancel:"true"` |
| Call mute | Same | Empty `kdeconnect.telephony.request_mute`; no outgoing call initiation is implemented |
| SMS read | `plugins/sms/SMSPlugin.kt`, `helpers/SMSHelper.kt` | `.sms.messages` version 2, `messages` array: `_id`, `thread_id`, `addresses:[{address}]`, `body`, numeric `date`, `type`, integer `read`, `event`; optional `sub_id`, attachments |
| SMS send | Same plus `plugins/sms/SmsMmsUtils.kt` | `.sms.request`: version 2, `addresses`, `messageBody`, empty `attachments`; handler reads **`subID`**, despite a comment describing `sub_id` |
| SMS history | `SMSPlugin.kt` | Empty `.sms.request_conversations`; `.sms.request_conversation` with `threadID`, optional `rangeStartTimestamp`, `numberToRequest` |
| MMS download | `SMSPlugin.kt`, `SmsMmsUtils.kt` | `.sms.request_attachment` uses `part_id`,`unique_identifier`; `.sms.attachment_file` responds with `filename` plus transport payload metadata |
| Contacts | `plugins/contacts/ContactsPlugin.kt`, `helpers/ContactsHelper.kt` | Empty `.contacts.request_all_uids_timestamps`; `.contacts.request_vcards_by_uid` with `uids`; responses `.contacts.response_uids_timestamps` and `.contacts.response_vcards` use `uids` plus entries keyed by UID |

The contacts implementation **does send inline vCard strings**, including its
metadata and any embedded card data. It does not attach a separate payload in the
inspected fork. `ContactCard.vcard` therefore retains the bounded opaque inline
card after checking its BEGIN/END envelope. It does not parse embedded photos,
follow URLs or import records. A payload-bearing contacts variant is rejected as
requiring a separate transfer handoff, rather than guessing an unsupported wire
format. A future interoperable payload variant needs its own verified contract.

The decoder accepts numeric-string timestamps/IDs where older peer descriptions
use them, and actual Android integer read flags plus boolean variants. It ignores
unknown optional fields and preserves SMS event bits. Inbound alphanumeric SMS
senders (such as a bank) are allowed; outbound recipients use a conservative phone
number grammar. Versions other than SMS v2 and legacy telephony SMS events are
explicitly unsupported.

## Android consent and SMS behavior

- Notifications require the Android notification-listener grant and app/plugin
  privacy choices. Content can be intentionally absent. Redacted notifications
  remain valid; no fallback extracts private content from icons or other fields.
- Contacts require Android `READ_CONTACTS` **and** its per-device
  `acceptedToTransferContacts` confirmation. No permission/confirmation is granted
  by this module or its tests.
- Telephony requires `READ_PHONE_STATE`/`READ_CALL_LOG`; contacts are optional.
  Phone numbers may be unavailable. Thumbnails are not decoded.
- SMSPlugin requires Android O or later and `SEND_SMS`, `READ_SMS`,
  `READ_PHONE_STATE`. Its normal KDE packet handler invokes the local Android
  SMS/MMS implementation; there is no gRPC API in these inspected sources.
- `send_sms` sends text-only structured JSON, with 1–10 explicit recipients and
  optional signed 32-bit `subID` (-1 means default). The phone may choose MMS for
  group/long messages; Android's MMS path requires being the default SMS app,
  whereas its plain SMS path does not. No attachment upload is invented here.
- The Android send path has no delivery-success reply. A successfully written
  packet is a **request sent**, never evidence of SMS delivery. Keep the reviewed
  recipients/body/SIM visible for final confirmation; do not automatically retry
  ambiguous sends. The module never opens a message dialog or sends a real SMS.
- Requesting SMS history enables subsequent Android message pushes. The inspected
  protocol has no unsubscribe packet. Disabling locally drops/forgets data;
  stopping those remote pushes requires the peer/plugin/connection lifecycle.

## Bounds and transfer handoff

| Boundary | Limit/behavior |
|---|---|
| Packet | Existing `Packet` validation and 64 KiB framed JSON limit, checked before typed decoding |
| Plain text/clipboard/SMS body | 16 KiB UTF-8; rejects non-text control characters; newlines/tabs remain text |
| Opaque IDs | 512 bytes, nonempty, no control characters; never interpreted as paths or commands |
| URL | 8192 bytes; HTTP(S), ASCII DNS/IP authority, valid port/percent escapes; no credentials, whitespace, backslashes or control characters |
| SMS batch | 128 messages, 32 inbound addresses per message, 16 attachment metadata entries per message |
| Outbound SMS | 10 recipients, 64-byte formatted numeric phone addresses; no shell metacharacters, comma-separated recipient injection or USSD/dial strings |
| Notification | 16 actions, 32 conversation lines; title 1024 bytes, app/sender labels 256 bytes; optional reply/icon identifiers bounded |
| Contact index/cards | 512 UIDs per reply; duplicate/reserved UID keys rejected; card 48 KiB and 2048 lines |
| Contact requests | At most 8 explicit UIDs per packet; request one at a time if cards contain photos |
| Cache | At most 128 events and 256 KiB of serialized event content; oldest evicted; element counts/string bounds also bound allocation overhead |
| Icon payload | Descriptor only, known size at most 1 MiB and nonzero 16-bit port |
| MMS payload | Descriptor only, known size at most 32 MiB and nonzero 16-bit port |

Contact index requests have no Android pagination, and several large inline
vCards/photos may exceed the existing frame limit. Oversized replies fail safely;
this module does not claim unlimited phonebook synchronization or invent paging.
The service must keep its existing frame-fatal handling when framing fails.

`PayloadDescriptor { size, port }` is a transfer offer, not a download instruction.
Notification icons and SMS files require the separate authenticated transfer
module, explicit destination/approval, sender-bound TLS identity, byte limits and
safe filename handling. `SmsAttachment.filename`/`unique_identifier` are opaque
untrusted labels, **never destination paths**. Do not follow a host supplied in
payload metadata; use the authenticated peer's network identity. Shared file
packets are handed off before interpreting `url`/`text`. Unrequested or disabled
payloads must be declined, not downloaded automatically.

## Verification

Only fabricated packets are used. The standalone test harness imports the existing
`packet.rs` and this new module without changing `lib.rs` or Cargo manifests.
Tests cover exact Android key spellings, redaction, cancellations/reply invalidation,
SMS numeric/read variants and missing fields, safe recipient formatting, literal
message data, opaque inline contact cards, payload descriptors, unknown fields,
collection/text/frame limits, default-off privacy and bounded cache eviction.
No real notifications, clipboard changes, messages, calls, contacts, ADB operations,
Android permission grants or device installs are performed by these tests.

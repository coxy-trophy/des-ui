# Spotify extensions — 2026-10-27

Implemented: URI-based library save/remove/contains compatibility; playlist creation,
name/description/privacy editing, native cover selection and JPEG upload; playlist
item add/remove/reorder; listening history; top tracks/artists with time ranges;
followed artists; album/playlist/show/audiobook saving and artist following; podcasts,
saved episodes, audiobook chapters; search pagination across supported content types.

Native chapter playback is not supported by the current backend. Selecting a chapter
opens its public Spotify page. Restricted recommendations/audio-analysis/related-artist
and editorial endpoints have not been enabled or advertised as supported.

## Verification

- `cargo test -p des-music -p des-ui --no-fail-fast`: 25 service tests and 503 shell
  tests passed. One native preview fixture is ignored during normal test runs.
- Executed that ignored fixture separately under Xvfb for wide (1100px), narrow
  (560px), playlist editor and compact (292px) views. All four passed; inspected
  each PNG in this directory. These are native egui renders with synthetic content.
- Tests cover URI validation, permission failure vs successful mutation responses,
  private playlist creation, current playlist-items bodies and snapshot IDs,
  cursor/range request paths, spoken content identity, server pagination offsets,
  duplicate page rejection, pagination errors, bounded PNG-to-JPEG cover encoding,
  compact volume accessibility and panel fit, and the complete existing shell suite.
- Existing desktop, terminal sessions and playback were not restarted. No account
  credentials were read by preview fixtures and no real playlists were modified.

## Still requires live account verification

The running installed binaries have not been replaced. Restarting/upgrading both
shell and music service is needed to expose the additions. Reconnect the library
through Spotify's own approval page to grant the seven additional scopes. Then:

1. Confirm Like/Unlike and collection save/remove/follow states against Spotify.
2. Create a disposable private playlist, edit details/privacy, upload a cover, add
   tracks/episodes including a duplicate, remove matching copies and reorder items.
   Confirm another device's edits are respected through snapshot IDs.
3. Browse history/top ranges/followed artists and load another page where available.
4. Browse/play a podcast episode and switch playback devices. Browse audiobook
   chapters and confirm opening Spotify; account/region restrictions may apply.
5. Search common and rare terms across pages; verify permission/rate-limit failures
   provide visible errors and a reconnect/retry route.

API contract sources checked against current Spotify documentation:
- https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide
- https://developer.spotify.com/documentation/web-api/reference/save-library-items
- https://developer.spotify.com/documentation/web-api/reference/get-users-top-artists-and-tracks
- https://developer.spotify.com/documentation/web-api/reference/get-recently-played
- https://developer.spotify.com/documentation/web-api/reference/create-playlist
- https://developer.spotify.com/documentation/web-api/reference/remove-items-playlist
- https://developer.spotify.com/documentation/web-api/reference/reorder-or-replace-playlists-items
- https://developer.spotify.com/documentation/web-api/reference/upload-custom-playlist-cover


## Bug review follow-up

`cargo test -p des-music -p des-ui --no-fail-fast`: **26 service + 510 shell tests
passed**, one optional native preview ignored in the normal suite. See
`bug-review-tests.log` for the fresh run.

Fixed the following review findings:

- Pagination stopped after unavailable-only pages because the UI keyed requests by
  visible row count; it now uses server offsets, keeps Load more available on empty
  filtered pages, and prevents duplicate in-flight requests. Retry resets that guard.
- Album pagination also advanced by visible rows; it now counts raw API entries.
- Further-page browsing errors used offset zero and discarded the accumulated list;
  errors retain their page offset and previously loaded rows. Invalid/repeated
  cursor continuation no longer loops over the first page.
- Playlist row positions could be paired with a separately fetched newer snapshot;
  page reads now check versions before/after and across continuation. The shell
  retains the displayed list's snapshot independently of editable metadata.
- Move up/down used adjacent raw positions instead of visible neighbours, so hidden
  items could make moves appear ineffective. Moves now target visible neighbours'
  original positions; boundary actions are disabled.
- Collaborators were offered owner-only name/privacy settings. Owner and item-edit
  permissions are now distinct. An empty add-to-playlist selector explains why.
- Refresh retained stale saved-state/permission metadata; it now invalidates all
  relevant caches. Like changes also invalidate subscribers' cached lists.
- Failed playlist saves/cover uploads discarded drafts. Each submission now has a
  distinct confirmation ID, fields remain stable while saving, errors retain drafts,
  unrelated/late confirmations cannot close another draft, and missing replies
  recover after 35 seconds without retrying mutations automatically.
- Cover reads could block opening a named pipe disguised as an image. Opens are
  nonblocking and reject non-regular files; a FIFO regression test covers this.
- Chapter links assumed a `/chapter/` URL, but Spotify can return episode identities.
  The canonical API URL is retained with URI-based fallback and browser-launch
  failures are reported through the existing opener.

Live-account checks and activation remain pending as described above. This review
uses synthetic fixtures and does not claim every possible defect has been eliminated.

Native review follow-up: the updated ignored fixture passed under Xvfb at 560px
for both `editor-error` and `editor-pending`. Inspected both PNGs: drafts remain
visible, long errors wrap inside the modal, and controls are disabled while saving.
These two fixture runs also avoid account access and service startup.

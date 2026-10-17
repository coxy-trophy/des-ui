# Handoff AA: the calendar under the clock, and notification upgrades

Done and wired on the branch; nothing is left for the parent to connect.

- `shell/src/agenda.rs` (new): a small RFC 5545 reader (folded lines, escaped text,
  DATE / local DATE-TIME / UTC `Z`, DURATION, SUMMARY, LOCATION, RRULE with
  FREQ=DAILY/WEEKLY/MONTHLY/YEARLY, INTERVAL, COUNT, UNTIL, BYDAY for weekly rules,
  EXDATE), expansion bounded to the window asked for, the month grid, the pop-up,
  and announcements fifteen minutes ahead. Calendars are read on one background thread
  every minute; `https://` / `webcal://` addresses in `calendars.list` are fetched with
  `curl` (10 s, 5 MiB, https only) at most every 30 minutes into
  `~/.cache/des-ui/calendars/`.
- Click the clock in the left column: the pop-up opens below it, is lifted above
  applications with the toasts' rectangle, and closes on Escape, a click elsewhere, or
  the lock. The status strip shows no clock, so there is nothing to click there.
- `notify.rs`: the list is grouped by application (most recent first, newest first
  inside, a header with a count that folds the group); `inline-reply` is advertised, a
  REPLY button opens a field in the list (from a toast too), and SEND emits
  `NotificationReplied(id, text)` and settles the notification; quiet hours
  (`dnd_schedule = 22:00-07:00`) keep all but urgent notifications in the list.
- Settings → Sound & screen → Notifications: a Quiet hours switch with from/to hours.

## Limits

- A time given with `TZID=` is taken as this machine's local time: `chrono-tz` is not in
  the lockfile and adding a time zone database was not worth it here.
- Monthly rules repeat on the day of the month; `BYDAY` positions such as `2TU` in
  monthly or yearly rules, `BYMONTHDAY`, `BYSETPOS` and `WKST` are not read.
- Evolution Data Server (GNOME Online Accounts calendars) is not read.

## README paragraph (under "What is in it")

> **Calendar.** Click the clock for a month calendar with today picked out; the day
> you choose lists its events. Put `.ics` files in `~/.config/des-ui/calendars/`, or
> list calendar addresses (`https://` or `webcal://`, one per line) in
> `~/.config/des-ui/calendars.list`; those are fetched at most every half hour. An
> event is announced fifteen minutes before it starts. Times in a named zone are
> taken as local time.
>
> **Notifications** are grouped by application in the list; click a heading to fold
> it. Applications that offer an answer get a REPLY field. **Settings → Sound &
> screen → Quiet hours** keeps all but urgent notifications in the list between the
> hours you choose.

## For docs/UNTESTED.md

- [ ] Click the clock: the calendar opens below it, today in the accent; arrows change
      month; a day with events has a dot; Escape and a click elsewhere close it
- [ ] With an application open, the pop-up shows above it (lifted like the toasts)
- [ ] A Google or Nextcloud calendar's `.ics` address in `calendars.list`: events show
      after the first fetch, and again after 30 minutes offline from the cache
- [ ] A weekly meeting with an exception and a monthly one on the 31st look right
- [ ] An event 14 minutes away is announced once
- [ ] Locale `en_US` starts weeks on Sunday, `en_GB` on Monday
- [ ] A chat application with inline reply (e.g. a KDE Connect SMS, Telegram Desktop):
      REPLY, type, SEND; the application receives the text
- [ ] Notifications from three applications: grouped, folding a group, CLEAR ALL
- [ ] Quiet hours across midnight: non-urgent notifications stay in the list, urgent
      ones still show; turning the switch off ends it at once

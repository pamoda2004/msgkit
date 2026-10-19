# Supported sources and their limits

msgkit only uses sources the platforms and the device owner permit. If a source is not listed as supported, it is because there is no legitimate way to obtain the data.

## WhatsApp

| Method | Messages | Calls | Notes |
|---|---|---|---|
| **Chat export** (`.txt`, "Export chat") | ✅ | ⚠️ missed-call notices only | User-initiated, one chat at a time |
| **Business Cloud API** webhooks | ✅ inbound | ❌ not yet | Needs a WhatsApp Business account and a verified webhook; you only see traffic to *your business number* |
| Personal account, programmatically | ❌ | ❌ | No official API. Libraries that automate WhatsApp Web/multi-device violate WhatsApp's terms and get accounts banned. msgkit will not add one. |

Export format caveats:
* Timestamps carry **no time zone**; pass `time_zone` (an exact IANA name such as `Asia/Colombo`, resolved with a database bundled in msgkit, daylight saving included) or, for a fixed offset, `utc_offset_minutes`. `time_zone` wins when both are given. A local time that does not exist (clocks forward) moves forward by the gap, a repeated one (clocks back) uses its first occurrence, and one warning counts both. Event ids never depend on the time zone.
* **Day/month order** differs by phone locale. `date_order: auto` infers it when any date has a day above 12; otherwise it assumes day-first and emits a warning. Set `dmy` or `mdy` explicitly for ambiguous files.
* Senders are **display names** (or numbers for non-contacts); the export has no stable ids.
* Time resolution is one minute (one second on iOS). Identical messages in the same minute get ordinal-disambiguated ids.
* Media is not included in the text file; attachments are recorded as references with the file name when available.
* Call history is not part of the export other than "Missed voice/video call" notices.
* **Known limitation: deleted and edited messages are not modelled.** A message deleted by its sender appears in the export as ordinary text (`This message was deleted`, `You deleted this message`), and msgkit imports that text as the message body, exactly like any other message. The `<This message was edited>` marker is removed from the body, and nothing records that the message was edited. Consumers that need to know should compare the body against these placeholder texts themselves. Modelling deletion and editing as lifecycle events is planned for Phase 6 and will be additive; the current behaviour is pinned by the conformance case `whatsapp_deleted_and_edited`.
* **Known limitation: English system notices only.** Lines without a sender are recognized as notices only in English. Notices in other languages are counted in a warning, never imported and never quoted.

## Messenger / Facebook

| Method | Messages | Calls | Notes |
|---|---|---|---|
| **Download Your Information** (JSON) | ✅ | ✅ (with duration) | Choose JSON format. Pass each `message_N.json` |
| **Messenger Platform** webhooks | ✅ | ❌ | Facebook **Pages** only; needs an app and a verified webhook |
| Personal inbox, programmatically | ❌ | ❌ | Not available through any official API |

Export caveats:
* Facebook encodes text as Latin-1-escaped UTF-8 ("mojibake"); msgkit repairs it.
* Messages are newest-first in the file; msgkit outputs oldest-first.
* Participants are names, not ids.
* Reactions, unsent messages and other special types are skipped.

## SMS and call log

| Platform | Support |
|---|---|
| **Android** | ✅ via a collector that reads `Telephony.Sms` and `CallLog.Calls` and emits a `DeviceBatch`. Requires `READ_SMS` / `READ_CALL_LOG`. See [ANDROID_COLLECTOR.md](ANDROID_COLLECTOR.md). |
| **iOS** | ❌ Apple provides no API to read messages or call history. |

Google Play restricts the SMS and call-log permissions to apps whose *core* function is default SMS/phone handling (or that qualify for a narrow exception). Apps distributed outside Play (enterprise, sideloaded, personal use) are not subject to this policy but must still request the runtime permissions.

## Not implemented yet

* WhatsApp Cloud API **call** webhooks.
* MMS attachments from the Android collector.
* Group-chat recipient resolution for exports (participants are not listed in WhatsApp exports).

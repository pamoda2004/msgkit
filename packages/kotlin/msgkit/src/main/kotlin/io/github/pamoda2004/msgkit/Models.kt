package io.github.pamoda2004.msgkit

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import java.time.Instant

// Types mirror the JSON Schemas in `schema/` of the msgkit repository
// (event.schema.json, import-report.schema.json, options.schema.json).

// <generated types: do not edit; run `python scripts/generate_types.py` after changing schema/>

/** The network or transport a message or call travelled over. */
@Serializable
public enum class Channel {
    /** Text messages (SMS/MMS) on a mobile device. */
    @SerialName("sms")
    SMS,

    /** Ordinary cellular voice calls. */
    @SerialName("cellular")
    CELLULAR,

    @SerialName("whatsapp")
    WHATSAPP,

    @SerialName("messenger")
    MESSENGER,
}

/** Whether the account owner received or sent the item. */
@Serializable
public enum class Direction {
    @SerialName("inbound")
    INBOUND,

    @SerialName("outbound")
    OUTBOUND,

    /** The source did not contain enough information to tell. */
    @SerialName("unknown")
    UNKNOWN,
}

/** How a call ended, from the account owner's point of view. */
@Serializable
public enum class CallOutcome {
    @SerialName("answered")
    ANSWERED,

    @SerialName("missed")
    MISSED,

    @SerialName("rejected")
    REJECTED,

    @SerialName("blocked")
    BLOCKED,

    @SerialName("voicemail")
    VOICEMAIL,

    @SerialName("unknown")
    UNKNOWN,
}

@Serializable
public enum class AttachmentKind {
    @SerialName("image")
    IMAGE,

    @SerialName("video")
    VIDEO,

    @SerialName("audio")
    AUDIO,

    @SerialName("file")
    FILE,

    @SerialName("other")
    OTHER,
}

/** A person or address taking part in a conversation. */
@Serializable
public data class Participant(
    /**
     * Stable identifier. An E.164 phone number (`+94700000000`) when one is known, otherwise the
     * platform's own id or the display name.
     */
    val id: String,
    @SerialName("display_name") val displayName: String? = null,
)

/** A media item or file belonging to a message. msgkit records references, never the bytes. */
@Serializable
public data class Attachment(
    val kind: AttachmentKind,
    /** Location of the media, if the source provided one (URL or export path). */
    val uri: String? = null,
    @SerialName("mime_type") val mimeType: String? = null,
    @SerialName("file_name") val fileName: String? = null,
    /** The platform's own media id, used to download the bytes via its API. */
    @SerialName("source_id") val sourceId: String? = null,
)

/** Anything msgkit can import: a message or a call. */
@Serializable
public sealed interface Event {
    /** Deterministic id: importing the same data twice yields the same id. */
    public val id: String
    public val channel: Channel
    public val direction: Direction

    /** ISO 8601, always UTC (for example `2024-01-01T00:00:00Z`). */
    public val timestamp: String
}

/** [Event.timestamp] as an [Instant] (needs Android API 26+, or core library desugaring). */
public val Event.instant: Instant get() = Instant.parse(timestamp)

/** A text or media message. */
@Serializable
@SerialName("message")
public data class Message(
    /** Deterministic id: importing the same data twice yields the same id. */
    override val id: String,
    override val channel: Channel,
    /** The id the source platform assigned, when it has one. */
    @SerialName("external_id") val externalId: String? = null,
    @SerialName("conversation_id") val conversationId: String? = null,
    override val direction: Direction,
    /** ISO 8601, always UTC (for example `2024-01-01T00:00:00Z`). */
    override val timestamp: String,
    val sender: Participant,
    val recipients: List<Participant> = emptyList(),
    val body: String? = null,
    val attachments: List<Attachment> = emptyList(),
    /** The untouched source payload (webhook events only), kept so records can be re-parsed later. */
    val raw: JsonElement? = null,
) : Event

/** A voice or video call. */
@Serializable
@SerialName("call")
public data class Call(
    override val id: String,
    override val channel: Channel,
    @SerialName("external_id") val externalId: String? = null,
    @SerialName("conversation_id") val conversationId: String? = null,
    override val direction: Direction,
    /** ISO 8601, always UTC (for example `2024-01-01T00:00:00Z`). */
    override val timestamp: String,
    /** Talk time in seconds; `0` for calls that were never answered. */
    @SerialName("duration_secs") val durationSecs: Long,
    val outcome: CallOutcome,
    val video: Boolean = false,
    val participants: List<Participant> = emptyList(),
    val raw: JsonElement? = null,
) : Event

/** The result of an import: the events plus anything worth telling the user. */
@Serializable
public data class ImportReport(
    val events: List<Event>,
    /** Non-fatal problems, e.g. lines that could not be parsed. */
    val warnings: List<String> = emptyList(),
) {
    public val messages: List<Message> get() = events.filterIsInstance<Message>()
    public val calls: List<Call> get() = events.filterIsInstance<Call>()
}

@Serializable
public enum class DateOrder {
    /** Infer from the file; falls back to day-first with a warning. */
    @SerialName("auto")
    AUTO,

    @SerialName("dmy")
    DMY,

    @SerialName("mdy")
    MDY,
}

/** Options for parsing a WhatsApp "Export chat" text file. */
@Serializable
public data class WhatsAppExportOptions(
    /**
     * The exporting user's name as it appears in the chat. Used to decide message direction; without
     * it direction is `unknown`.
     */
    @SerialName("self_name") val selfName: String? = null,
    /** Default `auto`: inferred from the data, falling back to day-first with a warning. */
    @SerialName("date_order") val dateOrder: DateOrder = DateOrder.AUTO,
    /**
     * Offset of the phone's local time zone from UTC, in minutes (e.g. `330` for UTC+05:30). Exports
     * carry no time zone. Defaults to `0`. Ignored when `time_zone` is set.
     */
    @SerialName("utc_offset_minutes") val utcOffsetMinutes: Int = 0,
    @SerialName("conversation_id") val conversationId: String? = null,
    /** Calling code (digits, e.g. `"94"`) used to normalize sender names that are national phone numbers. */
    @SerialName("default_country_code") val defaultCountryCode: String? = null,
    /**
     * Upper bounds for this import. The defaults suit real exports. Over a limit the import fails with
     * `limit_exceeded` and nothing is imported.
     */
    val limits: Limits = Limits(),
    /**
     * IANA time-zone name of the phone (exact spelling, e.g. `"Asia/Colombo"`). When set it replaces
     * `utc_offset_minutes` and follows the zone's daylight-saving rules; event ids do not depend on
     * it. A time in a repeated hour uses the first occurrence and a time in a skipped hour moves
     * forward by the gap (reported in one warning). The zone database is bundled with msgkit, so
     * results are the same everywhere.
     */
    @SerialName("time_zone") val timeZone: String? = null,
)

/** Options for parsing Messenger `message_N.json` files. */
@Serializable
public data class FacebookExportOptions(
    /** The exporting user's name, used to decide message direction. */
    @SerialName("self_name") val selfName: String? = null,
    /** Overrides the conversation id (defaults to the export's `thread_path`). */
    @SerialName("conversation_id") val conversationId: String? = null,
    /**
     * Upper bounds for this import. The defaults suit real exports. Over a limit the import fails with
     * `limit_exceeded` and nothing is imported.
     */
    val limits: Limits = Limits(),
)

/** A row from `Telephony.Sms`. */
@Serializable
public data class RawSms(
    /** Remote party: a phone number or an alphanumeric sender id. */
    val address: String,
    val body: String,
    /** Milliseconds since the Unix epoch. */
    @SerialName("date_ms") val dateMs: Long,
    /** `type` column: `1` inbox, `2` sent. Other values map to `unknown`. */
    @SerialName("type") val kind: Int,
    /** `_id` column, if available. Only stable on one device. */
    val id: String? = null,
    @SerialName("thread_id") val threadId: String? = null,
)

/** A row from `CallLog.Calls`. */
@Serializable
public data class RawCall(
    val number: String,
    @SerialName("date_ms") val dateMs: Long,
    /** `type` column: `1` incoming, `2` outgoing, `3` missed, `4` voicemail, `5` rejected, `6` blocked. */
    @SerialName("type") val kind: Int,
    @SerialName("duration_secs") val durationSecs: Long = 0,
    /** Cached contact name, if any. */
    val name: String? = null,
    val id: String? = null,
)

@Serializable
public data class DeviceBatch(
    val sms: List<RawSms> = emptyList(),
    val calls: List<RawCall> = emptyList(),
)

/** Options for normalizing Android SMS and call-log rows. */
@Serializable
public data class DeviceOptions(
    /** The device owner's own number, recorded as the other party of each message. Optional. */
    @SerialName("own_number") val ownNumber: String? = null,
    /** Calling code (digits) for national numbers such as `0700000000`. */
    @SerialName("default_country_code") val defaultCountryCode: String? = null,
    /**
     * Upper bounds for this import. The defaults suit real devices. Only `max_messages` (SMS rows plus
     * call rows) and the size and depth of the whole request apply to a batch. Over a limit the import
     * fails with `limit_exceeded`.
     */
    val limits: Limits = Limits(),
)

/**
 * Upper bounds for one import call; the call fails with `limit_exceeded` when one is passed. An absent
 * or null limit has its default. A limit must be at least 1 (0 is refused as `invalid_input`); to
 * switch the limits off set `unlimited` to true.
 */
@Serializable
public data class Limits(
    /**
     * Largest accepted input, in bytes: the exported text for a WhatsApp export, the whole JSON
     * document for the other imports. Absent: 33554432 (32 MiB).
     */
    @SerialName("max_input_bytes") val maxInputBytes: Long? = null,
    /**
     * Deepest accepted nesting of JSON arrays and objects in the document handed to msgkit. Through
     * the JSON entry point that document is the whole request, so its wrapper object counts. Absent:
     * 32.
     */
    @SerialName("max_json_depth") val maxJsonDepth: Long? = null,
    /** Longest accepted line of a WhatsApp export, in bytes. Absent: 262144 (256 KiB). */
    @SerialName("max_line_bytes") val maxLineBytes: Long? = null,
    /** Most messages, calls and notices accepted in one import. Absent: 1000000. */
    @SerialName("max_messages") val maxMessages: Long? = null,
    /** Switches every limit off. The only way to do so; use it only for input you trust. Default `false`. */
    val unlimited: Boolean = false,
)

// </generated types>

/** Versions of the native library and of the JSON data contract. */
@Serializable
public data class VersionInfo(
    @SerialName("crate_version") val crateVersion: String,
    @SerialName("schema_version") val schemaVersion: String,
    val operations: List<String> = emptyList(),
    /** Version of the bundled IANA time-zone database (for example `2026e`). */
    @SerialName("tzdata_version") val tzdataVersion: String = "unknown",
)

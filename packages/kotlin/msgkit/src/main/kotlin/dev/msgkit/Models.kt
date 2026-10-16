package dev.msgkit

import java.time.Instant
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

// Types mirror the JSON Schemas in `schema/` of the msgkit repository
// (event.schema.json, import-report.schema.json, options.schema.json).

/** The network or transport a message or call travelled over. */
@Serializable
public enum class Channel {
    @SerialName("sms") SMS,
    @SerialName("cellular") CELLULAR,
    @SerialName("whatsapp") WHATSAPP,
    @SerialName("messenger") MESSENGER,
}

/** Whether the account owner received or sent the item. */
@Serializable
public enum class Direction {
    @SerialName("inbound") INBOUND,
    @SerialName("outbound") OUTBOUND,
    @SerialName("unknown") UNKNOWN,
}

/** How a call ended, from the account owner's point of view. */
@Serializable
public enum class CallOutcome {
    @SerialName("answered") ANSWERED,
    @SerialName("missed") MISSED,
    @SerialName("rejected") REJECTED,
    @SerialName("blocked") BLOCKED,
    @SerialName("voicemail") VOICEMAIL,
    @SerialName("unknown") UNKNOWN,
}

@Serializable
public enum class AttachmentKind {
    @SerialName("image") IMAGE,
    @SerialName("video") VIDEO,
    @SerialName("audio") AUDIO,
    @SerialName("file") FILE,
    @SerialName("other") OTHER,
}

/** A person or address taking part in a conversation. */
@Serializable
public data class Participant(
    /** E.164 phone number when known, otherwise a platform id or display name. */
    val id: String,
    @SerialName("display_name") val displayName: String? = null,
)

/** A media item or file belonging to a message. msgkit records references, never bytes. */
@Serializable
public data class Attachment(
    val kind: AttachmentKind,
    val uri: String? = null,
    @SerialName("mime_type") val mimeType: String? = null,
    @SerialName("file_name") val fileName: String? = null,
    /** The platform's own media id, used to download the bytes via its API. */
    @SerialName("source_id") val sourceId: String? = null,
)

/** Anything msgkit can import: a [Message] or a [Call]. */
@Serializable
public sealed interface Event {
    /** Deterministic: importing the same data twice yields the same id. */
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
    override val id: String,
    override val channel: Channel,
    @SerialName("external_id") val externalId: String? = null,
    @SerialName("conversation_id") val conversationId: String? = null,
    override val direction: Direction,
    override val timestamp: String,
    val sender: Participant,
    val recipients: List<Participant> = emptyList(),
    val body: String? = null,
    val attachments: List<Attachment> = emptyList(),
    /** The untouched source payload (webhook events only). */
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
    override val timestamp: String,
    /** Talk time in seconds; 0 for calls that were never answered. */
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

/** Versions of the native library and of the JSON data contract. */
@Serializable
public data class VersionInfo(
    @SerialName("crate_version") val crateVersion: String,
    @SerialName("schema_version") val schemaVersion: String,
    val operations: List<String> = emptyList(),
)

// ---- Options and inputs ------------------------------------------------------

@Serializable
public enum class DateOrder {
    /** Inferred from the data; falls back to day-first with a warning. */
    @SerialName("auto") AUTO,
    @SerialName("dmy") DMY,
    @SerialName("mdy") MDY,
}

@Serializable
public data class WhatsAppExportOptions(
    /** The exporting user's name as shown in the chat; sets message direction. */
    @SerialName("self_name") val selfName: String? = null,
    @SerialName("date_order") val dateOrder: DateOrder = DateOrder.AUTO,
    /** Phone's UTC offset in minutes, e.g. 330 for UTC+05:30. Exports carry no time zone. */
    @SerialName("utc_offset_minutes") val utcOffsetMinutes: Int = 0,
    @SerialName("conversation_id") val conversationId: String? = null,
    /** Calling code (digits, e.g. "94") for national-format phone numbers. */
    @SerialName("default_country_code") val defaultCountryCode: String? = null,
)

@Serializable
public data class FacebookExportOptions(
    @SerialName("self_name") val selfName: String? = null,
    @SerialName("conversation_id") val conversationId: String? = null,
)

/** A row of Android's `Telephony.Sms`. */
@Serializable
public data class RawSms(
    val address: String,
    val body: String,
    /** Milliseconds since the Unix epoch. */
    @SerialName("date_ms") val dateMs: Long,
    /** 1 = inbox, 2 = sent. */
    @SerialName("type") val kind: Int,
    val id: String? = null,
    @SerialName("thread_id") val threadId: String? = null,
)

/** A row of Android's `CallLog.Calls`. */
@Serializable
public data class RawCall(
    val number: String,
    @SerialName("date_ms") val dateMs: Long,
    /** 1 incoming, 2 outgoing, 3 missed, 4 voicemail, 5 rejected, 6 blocked. */
    @SerialName("type") val kind: Int,
    @SerialName("duration_secs") val durationSecs: Long = 0,
    val name: String? = null,
    val id: String? = null,
)

@Serializable
public data class DeviceBatch(
    val sms: List<RawSms> = emptyList(),
    val calls: List<RawCall> = emptyList(),
)

@Serializable
public data class DeviceOptions(
    @SerialName("own_number") val ownNumber: String? = null,
    @SerialName("default_country_code") val defaultCountryCode: String? = null,
)

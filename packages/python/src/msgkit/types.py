"""Type definitions mirroring the JSON Schemas in ``schema/`` of the msgkit repository.

Values returned by msgkit are plain ``dict``/``list`` objects (so they
serialize with ``json`` and work with any library); these ``TypedDict`` classes
describe their shape for type checkers and editors.
"""

from __future__ import annotations

from typing import Any, List, Literal, Optional, TypedDict, Union

# <generated types: do not edit; run `python scripts/generate_types.py` after changing schema/>
Channel = Literal["sms", "cellular", "whatsapp", "messenger"]
Direction = Literal["inbound", "outbound", "unknown"]
CallOutcome = Literal["answered", "missed", "rejected", "blocked", "voicemail", "unknown"]
AttachmentKind = Literal["image", "video", "audio", "file", "other"]
DateOrder = Literal["auto", "dmy", "mdy"]


class _ParticipantRequired(TypedDict):
    # Stable identifier. An E.164 phone number (`+94700000000`) when one is known, otherwise the
    # platform's own id or the display name.
    id: str


class Participant(_ParticipantRequired, total=False):
    display_name: str


class _AttachmentRequired(TypedDict):
    kind: AttachmentKind


class Attachment(_AttachmentRequired, total=False):
    # Location of the media, if the source provided one (URL or export path).
    uri: str
    mime_type: str
    file_name: str
    # The platform's own media id, used to download the bytes via its API.
    source_id: str


class _MessageRequired(TypedDict):
    type: Literal["message"]
    # Deterministic id: importing the same data twice yields the same id.
    id: str
    channel: Channel
    direction: Direction
    # ISO 8601, always UTC (for example `2024-01-01T00:00:00Z`).
    timestamp: str
    sender: Participant
    recipients: List[Participant]
    attachments: List[Attachment]


class Message(_MessageRequired, total=False):
    # The id the source platform assigned, when it has one.
    external_id: str
    conversation_id: str
    body: str
    # The untouched source payload (webhook events only), kept so records can be re-parsed later.
    raw: Any


class _CallRequired(TypedDict):
    type: Literal["call"]
    id: str
    channel: Channel
    direction: Direction
    # ISO 8601, always UTC (for example `2024-01-01T00:00:00Z`).
    timestamp: str
    # Talk time in seconds; `0` for calls that were never answered.
    duration_secs: int
    outcome: CallOutcome
    video: bool
    participants: List[Participant]


class Call(_CallRequired, total=False):
    external_id: str
    conversation_id: str
    raw: Any


Event = Union[Message, Call]


class ImportReport(TypedDict):
    events: List[Event]
    # Non-fatal problems, e.g. lines that could not be parsed.
    warnings: List[str]


class WhatsAppExportOptions(TypedDict, total=False):
    # The exporting user's name as it appears in the chat. Used to decide message direction; without it
    # direction is `unknown`.
    self_name: Optional[str]
    # Default `auto`: inferred from the data, falling back to day-first with a warning.
    date_order: DateOrder
    # Offset of the phone's local time zone from UTC, in minutes (e.g. `330` for UTC+05:30). Exports carry
    # no time zone. Defaults to `0`. Ignored when `time_zone` is set.
    utc_offset_minutes: int
    conversation_id: Optional[str]
    # Calling code (digits, e.g. `"94"`) used to normalize sender names that are national phone numbers.
    default_country_code: Optional[str]
    # Upper bounds for this import. The defaults suit real exports. Over a limit the import fails with
    # `limit_exceeded` and nothing is imported.
    limits: Limits
    # IANA time-zone name of the phone (exact spelling, e.g. `"Asia/Colombo"`). When set it replaces
    # `utc_offset_minutes` and follows the zone's daylight-saving rules; event ids do not depend on it. A
    # time in a repeated hour uses the first occurrence and a time in a skipped hour moves forward by the
    # gap (reported in one warning). The zone database is bundled with msgkit, so results are the same
    # everywhere.
    time_zone: Optional[str]


class FacebookExportOptions(TypedDict, total=False):
    # The exporting user's name, used to decide message direction.
    self_name: Optional[str]
    # Overrides the conversation id (defaults to the export's `thread_path`).
    conversation_id: Optional[str]
    # Upper bounds for this import. The defaults suit real exports. Over a limit the import fails with
    # `limit_exceeded` and nothing is imported.
    limits: Limits


class _RawSmsRequired(TypedDict):
    # Remote party: a phone number or an alphanumeric sender id.
    address: str
    body: str
    # Milliseconds since the Unix epoch.
    date_ms: int
    # `type` column: `1` inbox, `2` sent. Other values map to `unknown`.
    type: int


class RawSms(_RawSmsRequired, total=False):
    # `_id` column, if available. Only stable on one device.
    id: Optional[str]
    thread_id: Optional[str]


class _RawCallRequired(TypedDict):
    number: str
    date_ms: int
    # `type` column: `1` incoming, `2` outgoing, `3` missed, `4` voicemail, `5` rejected, `6` blocked.
    type: int


class RawCall(_RawCallRequired, total=False):
    id: Optional[str]
    # Cached contact name, if any.
    name: Optional[str]
    duration_secs: int


class DeviceBatch(TypedDict, total=False):
    sms: List[RawSms]
    calls: List[RawCall]


class DeviceOptions(TypedDict, total=False):
    # The device owner's own number, recorded as the other party of each message. Optional.
    own_number: Optional[str]
    # Calling code (digits) for national numbers such as `0700000000`.
    default_country_code: Optional[str]
    # Upper bounds for this import. The defaults suit real devices. Only `max_messages` (SMS rows plus
    # call rows) and the size and depth of the whole request apply to a batch. Over a limit the import
    # fails with `limit_exceeded`.
    limits: Limits


class Limits(TypedDict, total=False):
    # Largest accepted input, in bytes: the exported text for a WhatsApp export, the whole JSON document
    # for the other imports. Absent: 33554432 (32 MiB).
    max_input_bytes: Optional[int]
    # Deepest accepted nesting of JSON arrays and objects in the document handed to msgkit. Through the
    # JSON entry point that document is the whole request, so its wrapper object counts. Absent: 32.
    max_json_depth: Optional[int]
    # Longest accepted line of a WhatsApp export, in bytes. Absent: 262144 (256 KiB).
    max_line_bytes: Optional[int]
    # Most messages, calls and notices accepted in one import. Absent: 1000000.
    max_messages: Optional[int]
    # Switches every limit off. The only way to do so; use it only for input you trust. Default `false`.
    unlimited: bool


# </generated types>


class VersionInfo(TypedDict):
    crate_version: str
    schema_version: str
    operations: List[str]
    tzdata_version: str  # version of the bundled IANA time-zone database, e.g. "2026e"

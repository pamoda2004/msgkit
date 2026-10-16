"""Type definitions mirroring the JSON Schemas in ``schema/`` of the msgkit repository.

Values returned by msgkit are plain ``dict``/``list`` objects (so they
serialize with ``json`` and work with any library); these ``TypedDict`` classes
describe their shape for type checkers and editors.
"""

from __future__ import annotations

from typing import Any, List, Literal, TypedDict, Union

Channel = Literal["sms", "cellular", "whatsapp", "messenger"]
Direction = Literal["inbound", "outbound", "unknown"]
CallOutcome = Literal["answered", "missed", "rejected", "blocked", "voicemail", "unknown"]
AttachmentKind = Literal["image", "video", "audio", "file", "other"]
DateOrder = Literal["auto", "dmy", "mdy"]


class _ParticipantRequired(TypedDict):
    id: str


class Participant(_ParticipantRequired, total=False):
    display_name: str


class _AttachmentRequired(TypedDict):
    kind: AttachmentKind


class Attachment(_AttachmentRequired, total=False):
    uri: str
    mime_type: str
    file_name: str
    source_id: str


class _MessageRequired(TypedDict):
    type: Literal["message"]
    id: str
    channel: Channel
    direction: Direction
    timestamp: str  # ISO 8601, always UTC
    sender: Participant
    recipients: List[Participant]
    attachments: List[Attachment]


class Message(_MessageRequired, total=False):
    external_id: str
    conversation_id: str
    body: str
    raw: Any


class _CallRequired(TypedDict):
    type: Literal["call"]
    id: str
    channel: Channel
    direction: Direction
    timestamp: str
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
    warnings: List[str]


class _RawSmsRequired(TypedDict):
    address: str
    body: str
    date_ms: int
    type: int  # 1 = inbox, 2 = sent


class RawSms(_RawSmsRequired, total=False):
    id: str
    thread_id: str


class _RawCallRequired(TypedDict):
    number: str
    date_ms: int
    type: int  # 1 incoming, 2 outgoing, 3 missed, 4 voicemail, 5 rejected, 6 blocked


class RawCall(_RawCallRequired, total=False):
    id: str
    name: str
    duration_secs: int


class DeviceBatch(TypedDict, total=False):
    sms: List[RawSms]
    calls: List[RawCall]


class VersionInfo(TypedDict):
    crate_version: str
    schema_version: str
    operations: List[str]

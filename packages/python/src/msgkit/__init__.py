"""msgkit: import SMS, WhatsApp and Messenger messages and calls into one normalized model.

    import msgkit

    report = msgkit.parse_whatsapp_export(open("_chat.txt", encoding="utf-8-sig").read(),
                                          self_name="Alice", utc_offset_minutes=330)
    for event in report["events"]:
        print(event["timestamp"], event["type"], event.get("body"))

Every function returns plain ``dict``/``list`` data whose shape is described by
:mod:`msgkit.types` and by the JSON Schemas in the msgkit repository, so the
data is identical in every language binding. Errors raise :class:`MsgkitError`.
"""

from __future__ import annotations

import json
from typing import Any, Mapping, Optional, Union, cast

from ._native import dispatch, native_version
from .errors import MsgkitError
from .types import DateOrder, DeviceBatch, ImportReport, VersionInfo

__version__ = "0.2.0"

__all__ = [
    "MsgkitError",
    "__version__",
    "dispatch",
    "native_version",
    "normalize_device",
    "normalize_messenger_webhook",
    "normalize_whatsapp_webhook",
    "parse_facebook_export",
    "parse_whatsapp_export",
    "verify_signature",
    "version",
]


def _options(**kwargs: Any) -> dict[str, Any]:
    """Drops unset (None) options so the library defaults apply."""
    return {k: v for k, v in kwargs.items() if v is not None}


def version() -> VersionInfo:
    """Versions of the native library and of the JSON data contract."""
    return cast(VersionInfo, dispatch("version"))


def parse_whatsapp_export(
    text: str,
    *,
    self_name: Optional[str] = None,
    date_order: Optional[DateOrder] = None,
    utc_offset_minutes: Optional[int] = None,
    conversation_id: Optional[str] = None,
    default_country_code: Optional[str] = None,
    time_zone: Optional[str] = None,
) -> ImportReport:
    """Parses a WhatsApp "Export chat" text file.

    ``self_name`` is the exporting user's name as shown in the chat and sets
    message direction. ``utc_offset_minutes`` is the phone's UTC offset (for
    example 330 for UTC+05:30); exports carry no time zone. ``time_zone`` is an
    exact IANA name such as ``"Asia/Colombo"``; it replaces ``utc_offset_minutes``
    and follows the zone's daylight-saving rules (an unknown name raises
    ``MsgkitError`` with code ``invalid_input``). ``date_order`` is ``"auto"``
    (default), ``"dmy"`` or ``"mdy"``.
    """
    options = _options(
        self_name=self_name,
        date_order=date_order,
        utc_offset_minutes=utc_offset_minutes,
        conversation_id=conversation_id,
        default_country_code=default_country_code,
        time_zone=time_zone,
    )
    return cast(ImportReport, dispatch("parse_whatsapp_export", {"text": text, "options": options}))


def parse_facebook_export(
    thread: Union[Mapping[str, Any], str],
    *,
    self_name: Optional[str] = None,
    conversation_id: Optional[str] = None,
) -> ImportReport:
    """Parses one Messenger ``message_N.json`` file ("Download Your Information").

    ``thread`` is the parsed JSON object or its text.
    """
    if isinstance(thread, str):
        thread = json.loads(thread)
    options = _options(self_name=self_name, conversation_id=conversation_id)
    return cast(ImportReport, dispatch("parse_facebook_export", {"thread": thread, "options": options}))


def normalize_device(
    batch: DeviceBatch,
    *,
    own_number: Optional[str] = None,
    default_country_code: Optional[str] = None,
) -> ImportReport:
    """Normalizes Android SMS and call-log rows collected on a device."""
    options = _options(own_number=own_number, default_country_code=default_country_code)
    return cast(ImportReport, dispatch("normalize_device", {"batch": batch, "options": options}))


def normalize_whatsapp_webhook(payload: Mapping[str, Any]) -> ImportReport:
    """Normalizes a WhatsApp Business Cloud API webhook payload.

    Does **not** verify the signature: call :func:`verify_signature` on the raw
    request body first.
    """
    return cast(ImportReport, dispatch("normalize_whatsapp_webhook", {"payload": payload}))


def normalize_messenger_webhook(payload: Mapping[str, Any]) -> ImportReport:
    """Normalizes a Messenger Platform webhook payload.

    Does **not** verify the signature: call :func:`verify_signature` on the raw
    request body first.
    """
    return cast(ImportReport, dispatch("normalize_messenger_webhook", {"payload": payload}))


def verify_signature(*, secret: str, body: Union[str, bytes], signature: str) -> bool:
    """Checks an ``X-Hub-Signature-256`` header against the raw, unparsed request body."""
    if isinstance(body, bytes):
        body = body.decode("utf-8")
    return bool(dispatch("verify_signature", {"secret": secret, "body": body, "signature": signature})["valid"])

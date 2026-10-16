class MsgkitError(Exception):
    """Error reported by msgkit.

    ``code`` is one of the stable codes documented in docs/API.md:
    ``invalid_input``, ``parse_error``, ``unknown_op``, ``storage_error``,
    ``internal_error``.
    """

    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message

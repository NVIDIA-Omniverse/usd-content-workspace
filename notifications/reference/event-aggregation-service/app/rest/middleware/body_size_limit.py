# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Pure-ASGI middleware bounding the size of an inbound REST request body."""

import json
import logging
from typing import Any, Awaitable, Callable, MutableMapping

logger = logging.getLogger(__name__)

Scope = MutableMapping[str, Any]
Message = MutableMapping[str, Any]
Receive = Callable[[], Awaitable[Message]]
Send = Callable[[Message], Awaitable[None]]
ASGIApp = Callable[[Scope, Receive, Send], Awaitable[None]]


class BodySizeLimitMiddleware:
    """
    Refuse a REST request whose body exceeds ``max_bytes`` with a 413.

    Two branches decide every ``http`` request. When ``content-length`` is
    present and parses as a non-negative integer it is authoritative: an
    oversize declaration is refused without reading a byte, and an acceptable
    one passes straight through to the inner application untouched. When it is
    absent or unusable (a chunked body, or a malformed value), the body is
    drained here first and the decision is made once the whole of it, or
    ``max_bytes`` of it, has been seen; the buffered messages are then replayed
    to the inner application, which is only ever invoked for a request already
    known to be within the cap.

    Trusting the declaration on the first branch relies on the ASGI server
    enforcing ``Content-Length`` framing. uvicorn does: with the ``standard``
    extra installed it parses with httptools/llhttp (``http="auto"`` resolves
    to ``HttpToolsProtocol``; h11 is only the fallback when httptools is
    absent), and both parsers frame the body strictly by the declared length
    and reject -- with a 400, before this application is invoked -- a
    duplicate ``Content-Length``, a negative or non-numeric one, and
    ``Content-Length`` combined with ``Transfer-Encoding: chunked``. Swapping
    the ASGI server out, or dropping the ``standard`` extra, means
    re-verifying that. The same shape has first-party precedent in
    ``notification_common``'s
    ``auth/auth_tokens/user_info_service_decoder.py`` (``_read_bounded_body``),
    where ``Content-Length`` is likewise a fast-path optimization and the
    streaming cap is the correctness primitive.

    Installed outermost (via ``add_middleware``, which inserts at index 0 of
    the user middleware list), it runs ahead of CORS, error handling, auth and
    tracing. An oversize request is therefore refused before it is
    authenticated, parsed, or published: the cap is a resource bound, not an
    authorization control.

    Memory promise: the middleware retains at most ``max_bytes`` of a request
    body. One further ASGI message may be transiently examined and discarded
    when it is the one that crosses the cap, and its size is bounded by the
    server's flow control: uvicorn accumulates the body in ``on_body`` and
    pauses reading on the transport once it exceeds its ``HIGH_WATER_LIMIT``
    of 64 KiB (``uvicorn/protocols/http/flow_control.py``), plus whatever the
    transport had already delivered before the pause took effect -- which is
    why ~128 KiB was observed in practice. Swapping the ASGI server out means
    re-verifying that bound against the replacement's flow-control constant.
    """

    def __init__(self, app: ASGIApp, max_bytes: int) -> None:
        """
        :param app: The inner ASGI application.
        :param max_bytes: The largest request body accepted, in bytes.
        """
        self.app = app
        self.max_bytes = max_bytes

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        """Apply the body-size bound to an ``http`` scope."""
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return

        declared_bytes = self._declared_body_bytes(scope)
        if declared_bytes is not None:
            if declared_bytes > self.max_bytes:
                logger.warning(
                    f"Refusing request to {scope.get('path')}: declared body size "
                    f"{declared_bytes} bytes exceeds the configured limit of "
                    f"{self.max_bytes} bytes"
                )
                await self._send_payload_too_large(send)
                return
            await self.app(scope, receive, send)
            return

        buffered: list[Message | None] = []
        observed_bytes = 0
        while True:
            message = await receive()
            if message["type"] != "http.request":
                # An ``http.disconnect`` ends the body; hand it on so the inner
                # application sees the peer going away.
                buffered.append(message)
                break
            observed_bytes += len(message.get("body", b""))
            if observed_bytes > self.max_bytes:
                logger.warning(
                    f"Refusing request to {scope.get('path')}: streamed body reached "
                    f"{observed_bytes} bytes, exceeding the configured limit of "
                    f"{self.max_bytes} bytes"
                )
                await self._send_payload_too_large(send)
                return
            buffered.append(message)
            if not message.get("more_body", False):
                break

        next_index = 0

        async def replay_receive() -> Message:
            """Hand back the drained messages, then defer to the real receive."""
            nonlocal next_index
            if next_index < len(buffered):
                message = buffered[next_index]
                # Drop the reference as it is handed over so the buffer does
                # not pin the whole body for the life of the request.
                buffered[next_index] = None
                next_index += 1
                return message
            return await receive()

        await self.app(scope, replay_receive, send)

    def _declared_body_bytes(self, scope: Scope) -> int | None:
        """
        Read the declared body size from the ``content-length`` header.

        :param scope: The ASGI connection scope.
        :return: The declared size in bytes, or None when the header is absent,
            duplicated, unparsable, or negative. None routes the request to the
            draining branch, which counts the body as it arrives.
        """
        declared: int | None = None
        for name, value in scope.get("headers", []):
            if name.lower() != b"content-length":
                continue
            if declared is not None:
                # Conflicting declarations are the classic smuggling
                # shape. Fall through to the counting branch rather
                # than trust either one.
                return None
            try:
                declared = int(value)
            except ValueError:
                return None
            if declared < 0:
                return None
        return declared

    async def _send_payload_too_large(self, send: Send) -> None:
        """
        Emit a 413 with the standard ``{"detail": ...}`` JSON error body.

        The response closes the connection. The refused body is never read by
        this application, so the server would otherwise sink the remainder of
        the declared body off the wire and let the client pipeline further
        requests on the same connection. RFC 9110 section 15.5.14 contemplates
        closing on a 413 for exactly that reason, and uvicorn honours the
        header: ``connection: close`` on a response start clears ``keep_alive``
        and the transport is closed once the body is written
        (``uvicorn/protocols/http/httptools_impl.py``, the response-start
        header loop and the response-completion branch). uvicorn's own 500 path
        emits the same header.

        :param send: The ASGI send callable.
        """
        body = json.dumps(
            {
                "detail": (
                    f"Request body exceeds the maximum of {self.max_bytes} "
                    f"bytes accepted by this service."
                )
            }
        ).encode()

        await send(
            {
                "type": "http.response.start",
                "status": 413,
                "headers": [
                    (b"content-type", b"application/json"),
                    (b"content-length", str(len(body)).encode()),
                    (b"connection", b"close"),
                ],
            }
        )
        await send({"type": "http.response.body", "body": body})

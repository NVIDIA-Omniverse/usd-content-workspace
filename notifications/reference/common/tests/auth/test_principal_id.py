# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import time
from contextvars import ContextVar

import jwt
import pytest

from notification_common.auth import principal_id
from notification_common.auth.auth_context import auth_token_var
from notification_common.auth.auth_tokens import (
    jwt_decoder,
    jwt_unverified_decoder,
    opaque_token_decoder,
    token_decoder,
    user_info_service_decoder,
)
from notification_common.auth.principal_source import PrincipalSource

# noinspection PyProtectedMember
from notification_common.auth.principal_id import (
    PrincipalContext,
    _PRINCIPAL_KEY,
    _principal_id_var,
    _cached_principal_id,
)
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError, ServerError

#: Signing material for the hand-built tokens below. Nothing verifies the
#: signature in `jwtUnverified` mode, but PyJWT warns below 32 bytes.
TOKEN_SECRET = "a-secret-long-enough-to-keep-pyjwt-quiet"

#: The principal that expired token claims.
EXPIRED_TOKEN_PRINCIPAL = "expired-principal"

#: The message principal_id() refuses a token with when it resolves to no
#: usable principal -- an absent claim, or one that is present but empty.
UNRESOLVED_MESSAGE = "Token did not resolve to a principal id"

#: Every principal source and the decoder module `decoder()` dispatches it to.
#: The refusal has to hold on all four: each decoder passes a present-but-empty
#: claim through as `""` rather than refusing it itself, so a path left out
#: here is a path that could still cache and publish an empty identity.
DECODER_MODULES_BY_SOURCE = [
    (PrincipalSource.JWT, jwt_decoder),
    (PrincipalSource.JWT_UNVERIFIED, jwt_unverified_decoder),
    (PrincipalSource.OPAQUE, opaque_token_decoder),
    (PrincipalSource.USER_INFO_SERVICE, user_info_service_decoder),
]


@pytest.fixture
def cached_user_id():
    return "cached-user"


@pytest.fixture
def mock_auth_enabled(mocker):
    """Mock authentication as enabled with the `jwt` principal source."""
    mock_settings = mocker.Mock()
    mock_settings.principal_source = PrincipalSource.JWT
    mock_settings.authentication_enabled.return_value = True
    mocker.patch.object(principal_id, "auth_settings", mock_settings)


@pytest.fixture
def exp():
    return time.time() + 300


@pytest.fixture
def decoder(mocker):
    # Use the real decoder() dispatch's selected backend module so the
    # call-count assertion lands on the actual client/decoder module.
    return mocker.patch.object(principal_id, "decoder", return_value=jwt_decoder)


@pytest.fixture
def get_user_id(exp, test_user_id, mocker):
    return mocker.patch.object(
        jwt_decoder, "get_user_id", return_value=(test_user_id, exp)
    )


@pytest.fixture
def jwt_unverified_source(mocker):
    # Patched on the shared settings object, which principal_id() and
    # decoder() both read, so the real dispatch selects the real
    # no-verify decoder.
    mocker.patch.object(
        auth_settings, "principal_source", PrincipalSource.JWT_UNVERIFIED
    )


@pytest.fixture
def set_up_token(request):
    # Seats the requesting class's TOKEN as the request's bearer token.
    # pytest-asyncio runs each test in its own task, so _principal_id_var
    # set during a call does not leak across tests; only the token var
    # needs an explicit reset.
    reset_token = auth_token_var.set(request.cls.TOKEN)
    yield
    auth_token_var.reset(reset_token)


@pytest.mark.asyncio
class TestGetPrincipalIdWhenAuthDisabled:
    @pytest.fixture(autouse=True)
    def mock_auth_disabled(self, mocker):
        """Mock authentication as disabled (principal_source == disabled)."""
        mock_settings = mocker.Mock()
        mock_settings.principal_source = PrincipalSource.DISABLED
        # authentication_enabled() is principal_source != disabled, so a
        # disabled source means it returns False.
        mock_settings.authentication_enabled.return_value = (
            mock_settings.principal_source != PrincipalSource.DISABLED
        )
        mocker.patch.object(principal_id, "auth_settings", mock_settings)

    async def test_returns_none_when_auth_disabled(self):
        """Test that principal_id returns None when authentication is disabled."""
        result = await principal_id.principal_id()
        assert result is None

    async def test_does_not_call_decoder_when_auth_disabled(self, mocker):
        """Test that decoder is not called when authentication is disabled."""
        decoder_mock = mocker.patch.object(principal_id, "decoder")

        await principal_id.principal_id()

        decoder_mock.assert_not_called()


@pytest.mark.asyncio
class TestGetPrincipalId:
    @pytest.fixture(autouse=True)
    def mock_auth_enabled(self, mocker):
        """Mock authentication as enabled for these tests."""
        mock_settings = mocker.Mock()
        mock_settings.authentication_enabled.return_value = True
        mocker.patch.object(principal_id, "auth_settings", mock_settings)

    @pytest.fixture(autouse=True)
    def set_up_token_and_mocks(self, decoder, get_user_id, test_token):
        reset_token = auth_token_var.set(test_token)
        yield
        auth_token_var.reset(reset_token)

    class TestWhenIdIsNotInCache:
        async def test_uses_decoder_func_to_get_decoder(self, decoder):
            await principal_id.principal_id()
            decoder.assert_called_once()

        async def test_uses_decoder_to_get_info(self, get_user_id):
            await principal_id.principal_id()
            get_user_id.assert_called_once()

        async def test_uses_token_from_context(self, get_user_id, test_token):
            await principal_id.principal_id()
            get_user_id.assert_called_once_with(test_token)

        async def test_sets_principal_id_in_context(self, test_user_id):
            await principal_id.principal_id()
            assert _principal_id_var.get()[0] == test_user_id

        async def test_sets_principal_exp_in_context(self, exp):
            await principal_id.principal_id()
            assert _principal_id_var.get()[1] == exp

    class TestWhenIdIsInCache:
        @pytest.fixture(autouse=True)
        def cache_token(self, cached_user_id, mocker):
            mocker.patch.object(
                principal_id, "_cached_principal_id", return_value=cached_user_id
            )

        async def test_returns_cached_token(
            self,
            cached_user_id,
        ):
            assert await principal_id.principal_id() == cached_user_id

        async def test_decoders_not_called(self, mocker):
            jwt_mock = mocker.patch.object(jwt_decoder, "get_user_id")
            opaque_mock = mocker.patch.object(opaque_token_decoder, "get_user_id")

            await principal_id.principal_id()

            jwt_mock.assert_not_called()
            opaque_mock.assert_not_called()


@pytest.mark.asyncio
class TestGetPrincipalIdCachesOncePerRequest:
    """
    Within a single request context, the selected backend is contacted at
    most once: the first principal_id() resolves and caches PrincipalContext,
    and a second call in the same context reuses the cached value without
    re-contacting the backend.

    This is also the witness for the ``is not None`` cache-hit test in
    principal_id(). Every principal that can now reach the cache is truthy --
    an empty one is refused before the cache write -- so there is no falsy
    cached value left to tell ``is not None`` apart from a truthiness test,
    and a row that seeded one would be pinning a state the code can no longer
    produce. What the check is there to do, serve a cached principal without a
    second decode, is what these rows assert.
    """

    @pytest.fixture(autouse=True)
    def set_up_token(self, mock_auth_enabled, decoder, get_user_id, test_token):
        # pytest-asyncio runs each test in its own task, so _principal_id_var
        # set during a call does not leak across tests; only the token var
        # needs an explicit reset.
        reset_token = auth_token_var.set(test_token)
        yield
        auth_token_var.reset(reset_token)

    async def test_backend_contacted_at_most_once(self, get_user_id):
        await principal_id.principal_id()
        await principal_id.principal_id()

        get_user_id.assert_called_once()

    async def test_second_call_returns_cached_value(self, test_user_id):
        first = await principal_id.principal_id()
        second = await principal_id.principal_id()

        assert first == test_user_id
        assert second == test_user_id


@pytest.mark.asyncio
@pytest.mark.usefixtures("jwt_unverified_source", "set_up_token")
class TestJwtUnverifiedResolvesAnExpiredTokenRepeatedly:
    """
    A token this mode accepts stays accepted for the whole request.

    ``jwtUnverified`` accepts an expired token on purpose: an upstream
    validator is the trust anchor and this mode makes no expiry judgement.
    Its decoder nevertheless used to report the *token's* own ``exp`` as the
    cache entry's expiry, so the cached principal was stale the instant it was
    written -- the first resolution succeeded, and every later one in the same
    request raised "Token expired" out of the cache. That turned a caller's
    single request into an accept-then-refuse depending on how many times
    something happened to resolve the principal, which is a decision the
    caching layer has no business making in a mode that judges no expiry.

    The whole path is real here -- the actual decoder, the actual cache -- so
    what is pinned is the behaviour and not a stubbed lifetime.
    """

    #: An already expired token, seated by the shared `set_up_token` fixture.
    TOKEN = jwt.encode(
        {"sub": EXPIRED_TOKEN_PRINCIPAL, "exp": 1},
        TOKEN_SECRET,
        algorithm="HS256",
    )

    async def test_the_first_resolution_returns_the_principal(self):
        assert await principal_id.principal_id() == EXPIRED_TOKEN_PRINCIPAL

    async def test_the_second_resolution_returns_the_principal(self):
        # The regression: this one used to raise AuthenticationError("Token
        # expired") from the cache the first call had just written.
        await principal_id.principal_id()

        assert await principal_id.principal_id() == EXPIRED_TOKEN_PRINCIPAL

    async def test_the_cache_entry_outlives_the_tokens_expiry(self):
        await principal_id.principal_id()

        assert _principal_id_var.get().expires_at > time.time()


@pytest.mark.asyncio
@pytest.mark.usefixtures("jwt_unverified_source", "set_up_token")
class TestJwtUnverifiedRefusesAnEmptyPrincipal:
    """
    A token whose principal claim is present but EMPTY is refused, exactly as
    an absent claim is, and nothing is written to the per-request cache.

    An empty claim carries no identity, so accepting it would stamp
    ``principal_identity=""`` on every event such a caller published and
    collapse all of them into one per-principal cache scope downstream. It is
    also the shape a misconfigured ``principal_id_claim`` produces, which the
    refusal surfaces instead of hiding behind an empty identity.

    The whole path is real here -- an actual token through the actual
    ``jwtUnverified`` decoder -- so what an empty claim does is observed end to
    end rather than assumed from a stub. The last row stubs only
    ``get_user_id``, to count backend contacts: refusing before the cache write
    is what makes the second call re-resolve instead of being served a cached
    ``""``.
    """

    #: A token whose principal claim is present but empty, seated by the shared
    #: `set_up_token` fixture.
    TOKEN = jwt.encode(
        {"sub": "", "exp": time.time() + 300},
        TOKEN_SECRET,
        algorithm="HS256",
    )

    @pytest.fixture
    def get_user_id(self, mocker):
        return mocker.patch.object(
            jwt_unverified_decoder, "get_user_id", return_value=("", time.time() + 300)
        )

    async def test_an_empty_claim_is_refused(self):
        with pytest.raises(AuthenticationError, match=UNRESOLVED_MESSAGE):
            await principal_id.principal_id()

    async def test_the_empty_principal_is_not_cached(self):
        with pytest.raises(AuthenticationError):
            await principal_id.principal_id()

        # Reading the cache directly, rather than inferring it from a second
        # call: a `""` written here would be served to any later in-service
        # caller in this context as a resolved principal.
        assert _cached_principal_id() is None

    async def test_a_second_call_re_resolves_rather_than_reading_a_cache(
        self, get_user_id
    ):
        for _ in range(2):
            with pytest.raises(AuthenticationError):
                await principal_id.principal_id()

        assert get_user_id.await_count == 2


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "principal_source, decoder_module",
    DECODER_MODULES_BY_SOURCE,
    ids=[source.value for source, _ in DECODER_MODULES_BY_SOURCE],
)
class TestEveryDecoderPathRefusesAnUnresolvedPrincipal:
    """
    The refusal is principal_id()'s, so it holds on every principal source.

    Each of the four decoders reads the principal out of its own backend --
    a verified JWT claim, an unverified one, an introspection response, a
    user-info response -- and each passes a present-but-empty value straight
    through as ``""``; only an absent one comes back as ``None``. None of them
    refuses the empty value itself, so the single refusal in principal_id() is
    what stands between an empty claim and a published ``principal_identity``
    of ``""``. A source that skipped it would be a deployment where the whole
    guarantee is off, which is why every source gets a row rather than the one
    the other classes here happen to exercise.

    Only that source's ``get_user_id`` is stubbed; the dispatch through
    ``decoder()`` is real, driven by the same ``principal_source`` setting a
    deployment sets.
    """

    @pytest.fixture(autouse=True)
    def selected_source(self, mocker, principal_source):
        # principal_id() and decoder() read principal_source off the one real
        # settings object, so this single patch points the real dispatch at
        # the parametrized backend.
        mocker.patch.object(auth_settings, "principal_source", principal_source)

    @pytest.fixture(autouse=True)
    def set_up_token(self, test_token):
        # pytest-asyncio runs each test in its own task, so _principal_id_var
        # set during a call does not leak across tests; only the token var
        # needs an explicit reset.
        reset_token = auth_token_var.set(test_token)
        yield
        auth_token_var.reset(reset_token)

    @pytest.fixture
    def resolve_to(self, mocker, decoder_module):
        """Make the parametrized source's backend resolve to ``resolved``."""

        def _patch(resolved):
            return mocker.patch.object(
                decoder_module, "get_user_id", return_value=resolved
            )

        return _patch

    async def test_an_empty_principal_is_refused(self, resolve_to, exp):
        resolve_to(("", exp))

        with pytest.raises(AuthenticationError, match=UNRESOLVED_MESSAGE):
            await principal_id.principal_id()

    async def test_an_empty_principal_is_not_cached(self, resolve_to, exp):
        resolve_to(("", exp))

        with pytest.raises(AuthenticationError):
            await principal_id.principal_id()

        assert _cached_principal_id() is None

    async def test_an_absent_principal_is_refused(self, resolve_to):
        # A decoder answers None when the claim (or the user-info `id` field)
        # is absent entirely. Without the refusal the tuple unpacking below it
        # would raise a raw TypeError, which reaches the caller as a 500.
        resolve_to(None)

        with pytest.raises(AuthenticationError, match=UNRESOLVED_MESSAGE):
            await principal_id.principal_id()

    async def test_a_none_principal_inside_a_tuple_is_refused(self, resolve_to, exp):
        # Defensive: no decoder produces this shape today. If one ever did,
        # caching it would make an authenticated request indistinguishable
        # from one where authentication is switched off, since that is the
        # other thing a None principal means.
        resolve_to((None, exp))

        with pytest.raises(AuthenticationError, match=UNRESOLVED_MESSAGE):
            await principal_id.principal_id()


@pytest.mark.asyncio
class TestGetPrincipalIdWhenUserInfoServiceUnreachable:
    """
    When principal_source is userInfoService and the user-info client is
    unreachable, the ServerError surfaces out of principal_id() unchanged:
    no None, no fallback to another backend.
    """

    @pytest.fixture(autouse=True)
    def mock_auth_enabled(self, mocker):
        mock_settings = mocker.Mock()
        mock_settings.principal_source = PrincipalSource.USER_INFO_SERVICE
        mock_settings.authentication_enabled.return_value = True
        mocker.patch.object(principal_id, "auth_settings", mock_settings)

    @pytest.fixture(autouse=True)
    def mock_decoder_dispatch(self, mocker):
        # Drive the real decoder() dispatch to the user-info client module by
        # pointing token_decoder's auth_settings at the userInfoService source.
        decoder_settings = mocker.Mock()
        decoder_settings.principal_source = PrincipalSource.USER_INFO_SERVICE
        mocker.patch.object(token_decoder, "auth_settings", decoder_settings)

    @pytest.fixture
    def unreachable_get_user_id(self, mocker):
        return mocker.patch.object(
            user_info_service_decoder,
            "get_user_id",
            side_effect=ServerError("user-info service unreachable: ClientError"),
        )

    @pytest.fixture(autouse=True)
    def set_up_token(self, test_token):
        reset_token = auth_token_var.set(test_token)
        yield
        auth_token_var.reset(reset_token)

    async def test_propagates_server_error(self, unreachable_get_user_id):
        with pytest.raises(ServerError):
            await principal_id.principal_id()

    async def test_contacts_user_info_client(self, unreachable_get_user_id):
        with pytest.raises(ServerError):
            await principal_id.principal_id()

        unreachable_get_user_id.assert_called_once()


@pytest.mark.asyncio
@pytest.mark.parametrize("principal_claim", ["sub", "oid"])
class TestCrossModePrincipalEquivalence:
    """
    Load-bearing for durable-queue-name stability: for the SAME token, the
    ``jwt`` (validating), ``jwtUnverified`` (no-verify), and ``userInfoService``
    paths must resolve to a byte-identical principal — the
    ``principal_id_claim``/``oid``-mapped value. A divergent claim read or a
    wrong user-info response field would silently orphan queues, so this asserts
    all three resolve equal, with the validating ``jwt`` result as the reference.

    Each path is driven through the real ``principal_id()`` dispatch by setting
    ``auth_settings.principal_source`` and patching only that path's backend with
    ``mocker.patch.object``. The per-request cache (``_principal_id_var``) is
    reset between modes so mode 2/3 cannot read mode 1's cached principal.

    Parametrized over the default ``sub`` claim and the ``oid`` override, since
    the user-info ``id`` -> claim mapping is the riskiest divergence point.
    """

    #: The principal value every backend is independently driven to produce.
    PRINCIPAL = "principal-from-claim"

    @pytest.fixture(autouse=True)
    def mock_auth_enabled(self, mocker, principal_claim):
        # principal_id() reads authentication_enabled() and principal_source off
        # this object; principal_source is flipped per-mode inside the test.
        mock_settings = mocker.Mock()
        mock_settings.authentication_enabled.return_value = True
        mock_settings.principal_id_claim = principal_claim
        mocker.patch.object(principal_id, "auth_settings", mock_settings)
        return mock_settings

    @pytest.fixture(autouse=True)
    def mock_decoder_dispatch(self, mocker, principal_claim):
        # decoder() dispatches on token_decoder.auth_settings.principal_source;
        # the test flips principal_source on this object per-mode.
        decoder_settings = mocker.Mock()
        decoder_settings.principal_id_claim = principal_claim
        mocker.patch.object(token_decoder, "auth_settings", decoder_settings)
        return decoder_settings

    @pytest.fixture(autouse=True)
    def restore_principal_var(self):
        # _resolve() swaps principal_id._principal_id_var for a fresh, unset
        # ContextVar to force a cache miss between modes; restore the original
        # module attribute afterwards so the swap never leaks to other tests.
        original = principal_id._principal_id_var
        yield
        principal_id._principal_id_var = original

    @pytest.fixture(autouse=True)
    def mock_claim_on_decoders(self, mocker, principal_claim):
        # The jwt / jwtUnverified get_user_id read principal_id_claim off their
        # module-level auth_settings; point both at the parametrized claim so the
        # equivalence exercises the real claim-reading logic, not a fixed key.
        jwt_settings = mocker.Mock()
        jwt_settings.principal_id_claim = principal_claim
        mocker.patch.object(jwt_decoder, "auth_settings", jwt_settings)

        unverified_settings = mocker.Mock()
        unverified_settings.principal_id_claim = principal_claim
        mocker.patch.object(
            jwt_unverified_decoder, "auth_settings", unverified_settings
        )

    @staticmethod
    def _reset_principal_cache():
        """
        Clear the per-request principal cache between modes.

        principal_id() sets _principal_id_var internally and discards the reset
        token, so the value cannot be reset back to "unset" from the test. The
        next-best reset that guarantees a cache MISS is to swap in a fresh,
        unset ContextVar; otherwise mode 2/3 would read mode 1's cached
        principal and the equivalence would pass vacuously.
        """
        fresh = ContextVar(_PRINCIPAL_KEY)
        principal_id._principal_id_var = fresh

    async def _resolve(self, source, mocker, test_token):
        """
        Reset the per-request cache, re-seat the token, point both the
        principal_id() and decoder() dispatch at ``source``, and resolve through
        the real principal_id() so every mode exercises the actual dispatch.
        """
        self._reset_principal_cache()
        reset_token = auth_token_var.set(test_token)

        principal_id.auth_settings.principal_source = source
        token_decoder.auth_settings.principal_source = source

        try:
            return await principal_id.principal_id()
        finally:
            auth_token_var.reset(reset_token)

    async def test_all_three_modes_resolve_equal_principal(
        self, mocker, test_token, principal_claim
    ):
        # jwt (validating): patch _decode_token so the real get_user_id reads the
        # parametrized claim out of the payload and returns it as the principal.
        mocker.patch.object(
            jwt_decoder,
            "_decode_token",
            return_value={principal_claim: self.PRINCIPAL, "exp": time.time() + 300},
        )
        jwt_principal = await self._resolve(PrincipalSource.JWT, mocker, test_token)

        # jwtUnverified: patch the no-verify _decode_token to yield the SAME
        # payload; real get_user_id reads the same claim key. _resolve() swaps in
        # a fresh _principal_id_var first, so this re-resolves through its own
        # backend rather than the cached jwt principal.
        mocker.patch.object(
            jwt_unverified_decoder,
            "_decode_token",
            return_value={principal_claim: self.PRINCIPAL, "exp": time.time() + 300},
        )
        unverified_principal = await self._resolve(
            PrincipalSource.JWT_UNVERIFIED, mocker, test_token
        )

        # userInfoService: patch the fetch so the response `id` field carries the
        # same principal; the decoder maps `id` -> principal.
        mocker.patch.object(
            user_info_service_decoder,
            "_fetch_userinfo_with_retry",
            return_value={"id": self.PRINCIPAL},
        )
        user_info_principal = await self._resolve(
            PrincipalSource.USER_INFO_SERVICE, mocker, test_token
        )

        # The validating jwt value is the reference; all three must be the same
        # byte-identical str.
        assert jwt_principal == self.PRINCIPAL
        assert unverified_principal == jwt_principal
        assert user_info_principal == jwt_principal

    async def test_diverging_user_info_field_breaks_equivalence(
        self, mocker, test_token, principal_claim
    ):
        # Proves the equivalence assertion is NOT vacuous: if the user-info
        # client returned a different `id` than the jwt claim value, the
        # userInfoService principal would differ from the jwt reference.
        mocker.patch.object(
            jwt_decoder,
            "_decode_token",
            return_value={principal_claim: self.PRINCIPAL, "exp": time.time() + 300},
        )
        jwt_principal = await self._resolve(PrincipalSource.JWT, mocker, test_token)

        mocker.patch.object(
            user_info_service_decoder,
            "_fetch_userinfo_with_retry",
            return_value={"id": "a-different-principal"},
        )
        user_info_principal = await self._resolve(
            PrincipalSource.USER_INFO_SERVICE, mocker, test_token
        )

        assert user_info_principal != jwt_principal


class TestCachedPrincipalId:
    class TestWhenAlreadyInCache:
        def test_returns_the_principal_id(self, test_user_id):
            reset_token = _principal_id_var.set(
                PrincipalContext(
                    principal_id=test_user_id, expires_at=time.time() + 300
                )
            )
            assert _cached_principal_id() == test_user_id
            _principal_id_var.reset(reset_token)

        def test_when_expired_it_raises_authentication_error(self, test_user_id):
            reset_token = _principal_id_var.set(
                PrincipalContext(principal_id=test_user_id, expires_at=1)
            )

            with pytest.raises(AuthenticationError, match="Token expired"):
                _cached_principal_id()

            _principal_id_var.reset(reset_token)

    class TestWhenNotAlreadyInCache:
        def test_returns_none(self):
            assert _cached_principal_id() is None

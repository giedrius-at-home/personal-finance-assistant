import hashlib
import hmac
import json
from concurrent.futures import ThreadPoolExecutor
from uuid import UUID, uuid4

import pytest

from finance_assistant.integrations.firefly_webhook import (
    FireflyWebhookReceiver,
    FireflyWebhookVerifier,
    InMemoryReplayGuard,
    ReplayCapacityError,
    WebhookAuthenticationError,
    WebhookPayloadError,
)

SECRET = "s" * 24
NOW = 1_800_000_000


def envelope(delivery_id: UUID | None = None) -> bytes:
    return json.dumps(
        {
            "uuid": str(delivery_id or uuid4()),
            "user_id": 1,
            "user_group_id": 1,
            "trigger": "STORE_TRANSACTION",
            "response": "TRANSACTIONS",
            "url": "http://gateway.test/v1/webhooks/firefly",
            "version": "v0",
            "content": {"synthetic": True},
        },
        separators=(",", ":"),
    ).encode()


def signature(body: bytes, timestamp: int = NOW, secret: str = SECRET) -> str:
    digest = hmac.new(
        secret.encode(),
        str(timestamp).encode() + b"." + body,
        hashlib.sha3_256,
    ).hexdigest()
    return f"t={timestamp},v1={digest}"


def test_verifier_accepts_a_matching_v1_candidate() -> None:
    body = envelope()
    verifier = FireflyWebhookVerifier(SECRET)
    valid = signature(body).split("v1=", 1)[1]

    assert (
        verifier.verify(
            body,
            f"v0={'0' * 64},t={NOW},v1={'1' * 64},v1={valid}",
            now=NOW,
        )
        == NOW
    )


@pytest.mark.parametrize(
    "header,now",
    [
        ("", NOW),
        ("t=not-a-time,v1=" + "0" * 64, NOW),
        (f"t={NOW},t={NOW},v1=" + "0" * 64, NOW),
        (f"t={NOW},v1=short", NOW),
        (f"t={NOW},v1=" + "0" * 64, NOW),
        (signature(b"{}", NOW - 301), NOW),
        (signature(b"{}", NOW + 301), NOW),
    ],
)
def test_verifier_rejects_malformed_invalid_and_stale_signatures(
    header: str, now: int
) -> None:
    body = b"{}"
    verifier = FireflyWebhookVerifier(SECRET)

    with pytest.raises(WebhookAuthenticationError) as captured:
        verifier.verify(body, header, now=now)

    assert SECRET not in str(captured.value)
    assert body.decode() not in str(captured.value)


def test_receiver_authenticates_before_parsing() -> None:
    receiver = FireflyWebhookReceiver(secret=SECRET, clock=lambda: NOW)

    with pytest.raises(WebhookAuthenticationError):
        receiver.receive(b"not-json", signature(b"different"))
    with pytest.raises(WebhookPayloadError):
        receiver.receive(b"not-json", signature(b"not-json"))


def test_receiver_claims_delivery_once() -> None:
    body = envelope()
    receiver = FireflyWebhookReceiver(secret=SECRET, clock=lambda: NOW)

    assert receiver.receive(body, signature(body)) is True
    assert receiver.receive(body, signature(body)) is False


def test_replay_claim_is_atomic() -> None:
    delivery_id = uuid4()
    guard = InMemoryReplayGuard()

    with ThreadPoolExecutor(max_workers=8) as executor:
        results = list(
            executor.map(
                lambda _: guard.claim(delivery_id, now=NOW),
                range(32),
            )
        )

    assert results.count(True) == 1
    assert results.count(False) == 31


def test_replay_guard_expires_entries_and_fails_closed_at_capacity() -> None:
    guard = InMemoryReplayGuard(retention_seconds=10, capacity=1)
    first = uuid4()
    second = uuid4()

    assert guard.claim(first, now=NOW)
    with pytest.raises(ReplayCapacityError):
        guard.claim(second, now=NOW + 1)
    assert guard.claim(second, now=NOW + 11)


@pytest.mark.parametrize("secret", ["", "too-short"])
def test_verifier_rejects_weak_secrets(secret: str) -> None:
    with pytest.raises(ValueError, match="24 bytes"):
        FireflyWebhookVerifier(secret)

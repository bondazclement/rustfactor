from __future__ import annotations

from dataclasses import dataclass

@dataclass(slots=True)
class StrikeEvidence:
    method: str
    reason: str
    before_ts_ms: int | None = None
    before_price: float | None = None
    after_ts_ms: int | None = None
    after_price: float | None = None
    used_ts_ms: int | None = None
    gap_ms: int | None = None


@dataclass(slots=True)
class StrikeComputation:
    value: float | None
    status: str  # exact_auto | approx_auto | pending_manual
    source: str
    confidence: float
    evidence: StrikeEvidence


def compute_strike_from_ticks(
    ticks: list[tuple[int, float]], event_start_ms: int,
    *,
    confidence_gap_ms: int,
    exact_match_tolerance_ms: int = 0,
) -> StrikeComputation:
    if not ticks:
        return StrikeComputation(
            value=None,
            status="pending_manual",
            source="rtds_missing_ticks",
            confidence=0.0,
            evidence=StrikeEvidence(method="none", reason="no_ticks"),
        )

    data = sorted((int(ts), float(px)) for ts, px in ticks)
    exact_candidates = [
        (ts, px) for ts, px in data if abs(ts - event_start_ms) <= exact_match_tolerance_ms
    ]
    if exact_candidates:
        ts, px = min(exact_candidates, key=lambda x: abs(x[0] - event_start_ms))
        return StrikeComputation(
            value=px,
            status="exact_auto",
            source="rtds_exact_tick",
            confidence=1.0,
            evidence=StrikeEvidence(
                method="exact_tick",
                reason="tick_at_boundary",
                used_ts_ms=ts,
                gap_ms=abs(ts - event_start_ms),
            ),
        )

    before = [(ts, px) for ts, px in data if ts < event_start_ms]
    after = [(ts, px) for ts, px in data if ts > event_start_ms]
    b = before[-1] if before else None
    a = after[0] if after else None

    if b and a:
        b_ts, b_px = b
        a_ts, a_px = a
        gap_ms = max(0, a_ts - b_ts)
        if gap_ms <= 0:
            est = a_px
        else:
            ratio = (event_start_ms - b_ts) / gap_ms
            ratio = max(0.0, min(1.0, ratio))
            est = b_px + (a_px - b_px) * ratio
        confidence = max(0.0, min(1.0, 1.0 - (gap_ms / max(1, confidence_gap_ms))))
        return StrikeComputation(
            value=est,
            status="approx_auto",
            source="rtds_interpolated_boundary",
            confidence=confidence,
            evidence=StrikeEvidence(
                method="interpolation",
                reason="before_after_boundary",
                before_ts_ms=b_ts,
                before_price=b_px,
                after_ts_ms=a_ts,
                after_price=a_px,
                gap_ms=gap_ms,
            ),
        )

    if a:
        a_ts, a_px = a
        dist = max(0, a_ts - event_start_ms)
        confidence = max(0.0, min(0.7, 1.0 - (dist / max(1, confidence_gap_ms))))
        return StrikeComputation(
            value=a_px,
            status="approx_auto",
            source="rtds_first_after",
            confidence=confidence,
            evidence=StrikeEvidence(
                method="first_after",
                reason="no_tick_before",
                after_ts_ms=a_ts,
                after_price=a_px,
                gap_ms=dist,
            ),
        )

    if b:
        b_ts, b_px = b
        dist = max(0, event_start_ms - b_ts)
        confidence = max(0.0, min(0.7, 1.0 - (dist / max(1, confidence_gap_ms))))
        return StrikeComputation(
            value=b_px,
            status="approx_auto",
            source="rtds_last_before",
            confidence=confidence,
            evidence=StrikeEvidence(
                method="last_before",
                reason="no_tick_after",
                before_ts_ms=b_ts,
                before_price=b_px,
                gap_ms=dist,
            ),
        )

    return StrikeComputation(
        value=None,
        status="pending_manual",
        source="rtds_unresolved",
        confidence=0.0,
        evidence=StrikeEvidence(method="none", reason="unresolved"),
    )

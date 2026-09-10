"""Presentation-only SVG coordinates; exact monetary values remain separate."""

from collections.abc import Sequence
from dataclasses import dataclass
from decimal import Decimal


@dataclass(frozen=True)
class ChartPoint:
    index: int
    value: Decimal
    x: Decimal
    y: Decimal


@dataclass(frozen=True)
class ChartSeries:
    segments: tuple[str, ...]
    points: tuple[ChartPoint, ...]


@dataclass(frozen=True)
class TrendChart:
    minimum: Decimal
    maximum: Decimal
    zero_y: Decimal
    series: tuple[ChartSeries, ...]

    @property
    def has_data(self) -> bool:
        return any(series.points for series in self.series)


def trend_chart(values: Sequence[Sequence[Decimal | None]]) -> TrendChart:
    present = [v for series in values for v in series if v is not None]
    minimum = min([Decimal(0), *present])
    maximum = max([Decimal(0), *present])
    if minimum == maximum:
        maximum = minimum + 1
    width = max((len(series) for series in values), default=1) - 1

    def y(value: Decimal) -> Decimal:
        return (20 + (maximum - value) * 180 / (maximum - minimum)).quantize(
            Decimal("0.01")
        )

    plotted = []
    for series in values:
        segments: list[str] = []
        segment: list[str] = []
        points = []
        for index, value in enumerate(series):
            if value is None:
                if segment:
                    segments.append(" ".join(segment))
                    segment = []
                continue
            x = (Decimal(40) + Decimal(index) * 520 / max(width, 1)).quantize(
                Decimal("0.01")
            )
            point = ChartPoint(index=index, value=value, x=x, y=y(value))
            points.append(point)
            segment.append(f"{point.x},{point.y}")
        if segment:
            segments.append(" ".join(segment))
        plotted.append(ChartSeries(segments=tuple(segments), points=tuple(points)))
    return TrendChart(
        minimum=minimum, maximum=maximum, zero_y=y(Decimal(0)), series=tuple(plotted)
    )

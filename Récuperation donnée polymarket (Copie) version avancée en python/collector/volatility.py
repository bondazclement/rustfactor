from __future__ import annotations

import math
import statistics
from collections import deque


class RollingVolatility:
    def __init__(self, max_ticks: int) -> None:
        self.prices: deque[float] = deque(maxlen=max_ticks)
        self.returns: deque[float] = deque(maxlen=max_ticks - 1 if max_ticks > 1 else 1)

    def push_price(self, price: float) -> None:
        if price <= 0:
            return
        if self.prices:
            prev = self.prices[-1]
            if prev > 0:
                self.returns.append(math.log(price / prev))
        self.prices.append(price)

    @property
    def spot(self) -> float | None:
        return self.prices[-1] if self.prices else None

    @property
    def inst_vol(self) -> float:
        if len(self.returns) < 2:
            return 0.0
        return statistics.pstdev(self.returns)

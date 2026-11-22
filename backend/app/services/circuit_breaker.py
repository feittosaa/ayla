import time


class CircuitBreaker:
    def __init__(self, max_failures=2, cooldown=20):
        self.max_failures = max_failures
        self.cooldown = cooldown
        self.failures = 0
        self.last_failure = None

    def allow(self) -> bool:
        if self.failures < self.max_failures:
            return True

        if time.time() - self.last_failure > self.cooldown:
            self.reset()
            return True

        return False

    def record_failure(self):
        self.failures += 1
        self.last_failure = time.time()

    def reset(self):
        self.failures = 0
        self.last_failure = None

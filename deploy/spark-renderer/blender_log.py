import re

EXCEPTION = re.compile(r"^(?:(?:[A-Za-z_]\w*\.)+\w*(?:Error|Exception|error)|[A-Za-z_]\w*(?:Error|Exception)): ")

WRAPPED = "Traceback (most recent call last):"


def cause(said: str) -> str:
    lines = [line.strip() for line in said.splitlines() if line.strip()]
    raised = [line for line in lines if EXCEPTION.match(line) and not line.endswith(WRAPPED)]
    return (raised or lines or ["no output"])[0 if raised else -1]

"""Finds running Roblox game processes by calling the RobloxProcessLookup helper.

The helper is the small C# program in csharp/RobloxProcessLookup. It prints a
JSON array, which this module parses. Windows only.
"""

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Optional


@dataclass
class RobloxProcess:
    pid: int
    start: str
    window_title: Optional[str]
    command_line: Optional[str]


def parse_processes(text: str) -> list[RobloxProcess]:
    """Turns the helper's JSON output into a list of RobloxProcess objects."""
    items = json.loads(text)
    return [
        RobloxProcess(
            pid=int(item["pid"]),
            start=item.get("start", ""),
            window_title=item.get("window_title"),
            command_line=item.get("command_line"),
        )
        for item in items
    ]


def list_roblox_processes(helper: Path) -> list[RobloxProcess]:
    """Runs the helper and returns the running Roblox game processes."""
    result = subprocess.run(
        [str(helper)],
        capture_output=True,
        text=True,
        timeout=15,
        check=True,
    )
    return parse_processes(result.stdout)


if __name__ == "__main__":
    import sys

    helper_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("RobloxProcessLookup.exe")
    for proc in list_roblox_processes(helper_path):
        print(proc)

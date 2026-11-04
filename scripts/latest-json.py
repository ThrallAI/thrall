"""Write latest.json, which installed copies of Thrall read to find an update.

Usage: latest-json.py <folder of release files> <tag> <notes file>

Each update file has a signature beside it (<file>.sig, made with Thrall's update key). An
installed copy downloads the file for its system and refuses it unless the signature matches.
The .deb isn't listed: Debian and Ubuntu update it through their package manager.
"""

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO = "ThrallAI/thrall"

# The ending of each system's update file, and the name the updater knows that system by.
PLATFORMS = {
    "windows-x86_64": "_x64-setup.exe",
    "linux-x86_64": "_amd64.AppImage",
    "darwin-aarch64": "_aarch64-apple-darwin.app.tar.gz",
    "darwin-x86_64": "_x86_64-apple-darwin.app.tar.gz",
}


def main() -> None:
    folder, tag, notes = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    platforms = {}
    for platform, ending in PLATFORMS.items():
        found = [f for f in folder.iterdir() if f.name.endswith(ending)]
        if len(found) != 1:
            sys.exit(f"expected one file ending {ending}, found {len(found)}")
        update = found[0]
        signature = update.with_name(update.name + ".sig")
        if not signature.is_file():
            sys.exit(f"{update.name} has no signature")
        platforms[platform] = {
            "signature": signature.read_text(encoding="utf-8").strip(),
            "url": f"https://github.com/{REPO}/releases/download/{tag}/{update.name}",
        }
    manifest = {
        "version": tag.removeprefix("v"),
        "notes": notes.read_text(encoding="utf-8").strip(),
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    (folder / "latest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()

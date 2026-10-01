#!/usr/bin/env python3
"""Post CI failure excerpts as a commit comment.

Usage: report-failures.py <log> [<log> ...]

Reads the given step logs (missing or passing ones are skipped), extracts
per-test failure sections when they exist, and writes a Markdown report to
stdout. Always emits something, even when every log is missing, so the
caller can post unconditionally. REPORT_TITLE and REPORT_OS name the
report. Logs may use LF or CRLF line endings and carry ANSI colors (CI
forces them on), both of which are normalized before matching.
"""

import os
import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")


def sections_for(text):
    """Per-test failure sections, or the bare tail when there are none."""
    match = re.search(r"^failures:\n((?:    .+\n)+)", text, re.M)
    names = (
        [line.strip() for line in match.group(1).splitlines() if line.strip()]
        if match
        else []
    )
    sections = []
    for name in names[:10]:
        section = re.search(
            r"^---- "
            + re.escape(name)
            + r" stdout ----\n(.*?)(?=^---- |\nfailures:|\ntest result:)",
            text,
            re.M | re.S,
        )
        if section:
            sections.append(f"### {name}\n```\n{section.group(1)[:12000]}\n```")
    if sections:
        return sections
    tail = "\n".join(text.splitlines()[-80:])
    return [f"(no per-test sections)\n```\n{tail[-12000:]}\n```"]


def main():
    chunks = []
    statuses = []
    unmatched = []
    for path in sys.argv[1:]:
        try:
            with open(path, errors="replace") as handle:
                raw = handle.read()
        except OSError:
            statuses.append(f"- {path}: missing")
            continue
        text = ANSI.sub("", raw.replace("\r\n", "\n"))
        if not text.strip():
            statuses.append(f"- {path}: empty")
            continue
        if "test result: FAILED" not in text and "\nerror" not in text:
            statuses.append(f"- {path}: {len(raw)} bytes, no failure markers")
            unmatched.append((path, text))
            continue
        statuses.append(f"- {path}: {len(raw)} bytes, matched")
        chunks.append(f"## {path}\n" + "\n".join(sections_for(text)))
    if chunks:
        body = "\n\n".join(chunks)
    else:
        lines = ["The failing step left no failure sections in its logs.", ""]
        lines.extend(statuses)
        if unmatched:
            path, text = unmatched[0]
            head = "\n".join(text.splitlines()[:5])[:1000]
            tail = "\n".join(text.splitlines()[-25:])[-4000:]
            lines += [
                "",
                f"## {path} (unmatched head/tail)",
                "```",
                head,
                "[...]",
                tail,
                "```",
            ]
        body = "\n".join(lines)
    title = os.environ.get("REPORT_TITLE", "Failures")
    os_name = os.environ.get("REPORT_OS", "?")
    sys.stdout.write(f"## {title} ({os_name})\n{body}"[:60000])


if __name__ == "__main__":
    main()

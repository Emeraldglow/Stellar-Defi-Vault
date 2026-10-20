#!/usr/bin/env python3
"""Gate CI on high-severity Scout findings (issue #664).

Reads the SARIF report produced by `cargo scout-audit --output-format sarif`,
prints a severity summary to stdout (and to the GitHub step summary when
available), and exits non-zero if any high-severity finding is not matched by
the tracked triage allowlist at ``.github/scout-triage.json``.

A finding is treated as "high severity" when any of these hold:

* SARIF ``level`` is ``error``;
* ``properties["security-severity"]`` parses as a number >= 7.0;
* ``properties["severity"]`` is one of high, critical, high/critical.

Usage:
    scout_triage.py SCOUT_SARIF [TRIAGE_JSON]
"""

from __future__ import annotations

import json
import os
import sys

HIGH_SEVERITY_NAMES = {"high", "critical", "high/critical", "high-critical"}
SECURITY_SEVERITY_FLOOR = 7.0


def load_json(path: str):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def severity_of(result: dict) -> str:
    """Return 'high', 'medium', 'low' or 'note' for a SARIF result."""
    props = result.get("properties") or {}

    raw = str(props.get("severity", "")).strip().lower()
    if raw in HIGH_SEVERITY_NAMES:
        return "high"

    sec = props.get("security-severity")
    if sec is not None:
        try:
            if float(sec) >= SECURITY_SEVERITY_FLOOR:
                return "high"
        except (TypeError, ValueError):
            pass
        return "medium"

    level = str(result.get("level", "")).strip().lower()
    if level == "error":
        return "high"
    if level == "warning":
        return "medium"
    return "low"


def location_of(result: dict) -> str:
    for loc in result.get("locations") or []:
        phys = loc.get("physicalLocation") or {}
        uri = (phys.get("artifactLocation") or {}).get("uri", "")
        line = (phys.get("region") or {}).get("startLine", "")
        if uri:
            return f"{uri}:{line}" if line else uri
    return "<unknown>"


def message_of(result: dict) -> str:
    msg = result.get("message")
    if isinstance(msg, dict):
        return str(msg.get("text", "")).strip()
    return str(msg or "").strip()


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: scout_triage.py SCOUT_SARIF [TRIAGE_JSON]", file=sys.stderr)
        return 2

    sarif_path = sys.argv[1]
    triage_path = sys.argv[2] if len(sys.argv) > 2 else ".github/scout-triage.json"

    if not os.path.isfile(sarif_path):
        # Scout writes its report under a name that varies by release; fall back
        # to the single SARIF file in the tree (ignoring build output).
        candidates = []
        for root, dirs, files in os.walk("."):
            dirs[:] = [d for d in dirs if d not in {"target", ".git"}]
            candidates += [
                os.path.join(root, name) for name in files if name.endswith(".sarif")
            ]
        if len(candidates) != 1:
            print(
                f"expected exactly one SARIF report, found {candidates!r}",
                file=sys.stderr,
            )
            return 2
        sarif_path = candidates[0]
        print(f"using SARIF report: {sarif_path}")

    try:
        sarif = load_json(sarif_path)
        triage = load_json(triage_path)
    except (OSError, json.JSONDecodeError) as exc:
        print(f"failed to read a report file: {exc}", file=sys.stderr)
        return 2
    allowed = triage.get("allowed", [])

    results = []
    for run in sarif.get("runs", []):
        results.extend(run.get("results", []))

    counts = {"high": 0, "medium": 0, "low": 0}
    unapproved_high = []

    for result in results:
        severity = severity_of(result)
        counts[severity] += 1
        if severity != "high":
            continue
        rule_id = result.get("ruleId") or "<no-rule>"
        message = message_of(result)
        # A high finding is suppressed only by an allowlist entry with the same
        # ruleId, an optional message substring, and a non-empty justification.
        approved = any(
            entry.get("ruleId") == rule_id
            and (
                not entry.get("match")
                or entry["match"].lower() in message.lower()
            )
            and str(entry.get("justification", "")).strip()
            for entry in allowed
        )
        if not approved:
            unapproved_high.append((rule_id, location_of(result), message))

    lines = [
        "### Scout static analysis",
        "",
        f"- High severity: **{counts['high']}** (of which {len(unapproved_high)} untriaged)",
        f"- Medium severity: {counts['medium']}",
        f"- Low / note: {counts['low']}",
        "",
    ]
    if unapproved_high:
        lines.append("#### Untriaged high-severity findings")
        lines.append("")
        for rule_id, location, message in unapproved_high:
            lines.append(f"- `{rule_id}` at `{location}` — {message}")
        lines.append("")
        lines.append(
            "Fix the finding, or add a justified entry to "
            "`.github/scout-triage.json` (see `docs/STATIC_ANALYSIS.md`)."
        )
    else:
        lines.append("No untriaged high-severity findings.")

    report = "\n".join(lines)
    print(report)
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as handle:
            handle.write(report + "\n")

    return 1 if unapproved_high else 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Compare clean Python rootfs build reports by target platform."""

import argparse
import json
import pathlib
import sys


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected-replicas", type=int, default=2)
    parser.add_argument("--expected-targets", type=int, default=2)
    parser.add_argument("reports", nargs="+")
    args = parser.parse_args()

    grouped: dict[str, list[dict]] = {}
    errors: list[str] = []
    for report_name in args.reports:
        report = json.loads(pathlib.Path(report_name).read_text())
        target = report.get("target_platform", "<missing>")
        grouped.setdefault(target, []).append(report)
        if report.get("status") != "ok" or not report.get("equal"):
            errors.append(f"{report_name}: the local clean-build comparison failed")
        if report.get("no_build_cache") is not True:
            errors.append(f"{report_name}: build cache was not disabled")

    if len(grouped) != args.expected_targets:
        errors.append(
            f"expected {args.expected_targets} target platforms, found {len(grouped)}"
        )

    targets = []
    for target, reports in sorted(grouped.items()):
        if len(reports) != args.expected_replicas:
            errors.append(
                f"{target}: expected {args.expected_replicas} clean runners, found {len(reports)}"
            )
        identities = [report.get("identities") for report in reports]
        equal = bool(identities) and all(value == identities[0] for value in identities[1:])
        if not equal:
            errors.append(f"{target}: identities differ across clean runners")
        targets.append(
            {
                "target_platform": target,
                "runner_count": len(reports),
                "equal": equal,
                "identities": identities[0] if equal else None,
                "build_hosts": [report.get("build_host") for report in reports],
                "builders": [report.get("builder") for report in reports],
            }
        )

    result = {
        "schema_version": 1,
        "status": "ok" if not errors else "mismatch",
        "expected_replicas": args.expected_replicas,
        "expected_targets": args.expected_targets,
        "targets": targets,
        "errors": errors,
    }
    json.dump(result, sys.stdout, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())

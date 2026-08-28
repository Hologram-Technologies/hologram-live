#!/usr/bin/env python3
"""Compare clean Python Component build reports by native build host."""

import argparse
import json
import pathlib
import sys


EXPECTED_TARGETS = {
    "linux/aarch64",
    "linux/x86_64",
    "macos/aarch64",
    "macos/x86_64",
    "windows/x86_64",
}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected-replicas", type=int, default=2)
    parser.add_argument("reports", nargs="+")
    args = parser.parse_args()

    grouped: dict[str, list[dict]] = {}
    errors: list[str] = []
    for report_name in args.reports:
        report = json.loads(pathlib.Path(report_name).read_text())
        target = report.get("target_host", "<missing>")
        grouped.setdefault(target, []).append(report)
        if report.get("status") != "ok" or not report.get("equal"):
            errors.append(f"{report_name}: the isolated component build failed")
        if report.get("isolated_uv_cache") is not True:
            errors.append(f"{report_name}: the uv cache was not isolated")
        if report.get("build_count") != 1:
            errors.append(f"{report_name}: each clean runner must contribute exactly one build")

    actual_targets = set(grouped)
    if actual_targets != EXPECTED_TARGETS:
        missing = sorted(EXPECTED_TARGETS - actual_targets)
        unexpected = sorted(actual_targets - EXPECTED_TARGETS)
        errors.append(
            f"host matrix mismatch; missing={missing!r}, unexpected={unexpected!r}"
        )

    targets = []
    for target, reports in sorted(grouped.items()):
        if len(reports) != args.expected_replicas:
            errors.append(
                f"{target}: expected {args.expected_replicas} clean runners, found {len(reports)}"
            )
        identities = [report.get("identities") for report in reports]
        contracts = [report.get("build_contract") for report in reports]
        equal = bool(identities) and all(value == identities[0] for value in identities[1:])
        same_contract = bool(contracts) and all(value == contracts[0] for value in contracts[1:])
        if not equal:
            errors.append(f"{target}: canonical or physical identities differ across clean runners")
        if not same_contract:
            errors.append(f"{target}: build contracts differ across clean runners")
        targets.append(
            {
                "target_host": target,
                "runner_count": len(reports),
                "equal": equal,
                "same_contract": same_contract,
                "identities": identities[0] if equal else None,
                "build_hosts": [report.get("build_host") for report in reports],
                "provenance_reproducible": [
                    report.get("provenance_reproducible") for report in reports
                ],
            }
        )

    result = {
        "schema_version": 1,
        "status": "ok" if not errors else "mismatch",
        "expected_replicas": args.expected_replicas,
        "expected_targets": sorted(EXPECTED_TARGETS),
        "target_local_equality": not errors,
        "targets": targets,
        "errors": errors,
    }
    json.dump(result, sys.stdout, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())

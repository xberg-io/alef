"""Remove precisely the documented source-level allowance, preserving other flags."""

import shlex

ALLOWANCE = "link-arg=-Wl,-undefined,dynamic_lookup"


def strict_flags(environment, configured):
    if environment.get("CARGO_ENCODED_RUSTFLAGS") is not None:
        value = environment["CARGO_ENCODED_RUSTFLAGS"]
        flags = value.split("\x1f") if value else []
    elif environment.get("RUSTFLAGS") is not None:
        flags = shlex.split(environment["RUSTFLAGS"])
    else:
        flags = list(configured)
    result = []
    removed = 0
    index = 0
    while index < len(flags):
        if flags[index : index + 2] == ["-C", ALLOWANCE]:
            removed += 1
            index += 2
        elif flags[index] == "-C" + ALLOWANCE:
            removed += 1
            index += 1
        else:
            if "dynamic_lookup" in flags[index]:
                raise ValueError("unrecognized global allowance spelling requires explicit review")
            result.append(flags[index])
            index += 1
    # rustc repeats `-lc++`/`-lobjc` once per native crate, and Xcode 15+ ld reports the repeats, so every
    # cdylib in the consumer graph (xberg-ffi) would fail on rustc's own link line. Only that warning is
    # silenced; the SwiftPM duplicate-library control in negative.py still runs fatal. ~keep
    for strict in (["-C", "link-arg=-Wl,-fatal_warnings"], ["-C", "link-arg=-Wl,-no_warn_duplicate_libraries"]):
        if not any(result[i : i + 2] == strict for i in range(len(result))):
            result.extend(strict)
    return result, removed

"""D4's verdict over owned_grow_replay.c's rows (ADR 16.10.26j).

Input: one line per (policy, shape, pad) from `owned_grow_replay`, as
`MODE=replay` writes them: policy piece seeds pad calls moves moves/round
bytes_kept ns/round.

A shape's move count depends on the heap's starting offset (the pad) as much
as on the policy, so each (policy, shape) is summarised by its mean over the
pads, and D4's rule reads the means: a policy qualifies when, on every shape,
`calls + w * moves` is at most Rust's for w = 1 and w = 4. The smallest
qualifying `geo` factor is the pick. The strict per-offset reading (the same
comparison at each pad) is counted beside each verdict, for the record.
"""

import collections
import statistics
import sys

WEIGHTS = (1, 4)
ROUNDS = 200_000


def work(calls, moves, weight):
    return calls + weight * moves


def main(path):
    rows = [line.split() for line in open(path, encoding="utf-8") if line.strip()]
    cells = collections.defaultdict(list)
    for row in rows:
        cells[row[0], row[1], row[2]].append(row)
    mean = {
        cell: [statistics.mean(float(row[i]) for row in group) for i in (4, 5, 7, 8)]
        for cell, group in cells.items()
    }
    pads = len(rows) // len(cells)
    print(f"== mean over {pads} offsets: policy piece seeds calls moves moves/round bytes_kept ns/round")
    for (policy, piece, seeds), (calls, moves, kept, ns) in mean.items():
        print(f"{policy} {piece} {seeds} {calls:.0f} {moves:.0f} {moves / ROUNDS:.2f} {kept:.0f} {ns:.1f}")

    rust_at = {(row[1], row[2], row[3]): (int(row[4]), int(row[5])) for row in rows if row[0] == "rust"}
    print("== D4: calls + w * moves at most Rust's on every shape's mean, w = 1 and w = 4")
    picked = None
    for policy in dict.fromkeys(row[0] for row in rows if row[0] != "rust"):
        fails = [
            f"{piece}/{seeds} w={weight}"
            for (name, piece, seeds), (calls, moves, _, _) in mean.items()
            if name == policy
            for weight in WEIGHTS
            if work(calls, moves, weight) > work(*mean["rust", piece, seeds][:2], weight)
        ]
        strict = sum(
            1
            for row in rows
            if row[0] == policy
            for weight in WEIGHTS
            if work(int(row[4]), int(row[5]), weight) > work(*rust_at[row[1], row[2], row[3]], weight)
        )
        cells_per_policy = len(WEIGHTS) * sum(1 for row in rows if row[0] == policy)
        verdict = "qualifies" if not fails else "fails " + ", ".join(fails)
        print(f"  {policy}: {verdict} (per offset: {strict} of {cells_per_policy} cells above Rust)")
        if not fails and policy.startswith("geo") and picked is None:
            picked = policy
    print(f"  pick: {picked[3:] if picked else 'none qualifies (D8.3)'}")


if __name__ == "__main__":
    main(sys.argv[1])

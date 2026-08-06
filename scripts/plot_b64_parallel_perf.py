#!/usr/bin/env python3
"""Plot b64 parallel perf from results.json (no benchmark rerun)."""
import json
import sys
from pathlib import Path

# import plot_results from sibling module
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_b64_parallel_perf import plot_results  # noqa: E402

if __name__ == "__main__":
    json_path = Path(sys.argv[1] if len(sys.argv) > 1 else "results.json")
    outdir = Path(sys.argv[2] if len(sys.argv) > 2 else json_path.parent)
    data = json.loads(json_path.read_text(encoding="utf-8"))
    plot_results(data["runs"], outdir)
    print("done", outdir)
